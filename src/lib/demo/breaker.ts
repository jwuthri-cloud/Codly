// Mock circuit breaker for `pnpm dev` and the site demo: the same steer → constrain → stop ladder as
// the core's, driven by the signals the scripted story really has, work tokens and cost. Set a
// budget below what the team has used (Settings → Autonomy & Budgets) and the working agents are
// steered, then constrained, and with Hard stop stopped, each step a decision on their timeline.
// The scripted story starts its tasks itself: unlike the core, the mock does not hold them.
import type { Agent } from '../gen/Agent';
import type { AgentBreaker } from '../gen/AgentBreaker';
import type { BreakerConfig } from '../gen/BreakerConfig';
import type { BreakerLevel } from '../gen/BreakerLevel';
import type { BreakerStatus } from '../gen/BreakerStatus';
import type { Spent } from '../gen/Spent';
import type { Sim } from './sim';

const DEFAULT: BreakerConfig = { enabled: true, token_budget: 0, cost_budget_usd: 0, velocity_per_min: 60_000, repeated_tool_limit: 8, error_storm_limit: 5, hard_stop: false };
const NEXT: BreakerLevel[] = ['healthy', 'steering', 'constrained', 'stopped'];

export class MockBreaker {
  cfg: BreakerConfig = { ...DEFAULT };
  private levels = new Map<string, BreakerLevel>();
  /** What the team had used at the last Reset. */
  private baseline: Spent = { tokens: 0, cost_usd: 0 };
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(private sim: Sim) {}

  private work(a: { usage: { input_tokens: number; output_tokens: number; cache_write_tokens: number } }) {
    return a.usage.input_tokens + a.usage.output_tokens + a.usage.cache_write_tokens;
  }
  private sum(agents: Iterable<Agent>): Spent {
    const total = { tokens: 0, cost_usd: 0 };
    for (const a of agents) {
      total.tokens += this.work(a);
      total.cost_usd += a.usage.cost_usd ?? 0;
    }
    return total;
  }
  /** Everything the story's agents used: the demo is one day long. */
  private today(): Spent {
    return this.sum(this.sim.agents.values());
  }
  /** Used since the last Reset: what the budgets are checked against. */
  private used(): Spent {
    const t = this.today();
    return { tokens: Math.max(0, t.tokens - this.baseline.tokens), cost_usd: Math.max(0, t.cost_usd - this.baseline.cost_usd) };
  }
  /** The budget the team is over, in the core's words. */
  private over(): { trip: string; why: string } | null {
    const c = this.cfg;
    const u = this.used();
    if (!c.enabled) return null;
    if (c.token_budget > 0 && u.tokens > c.token_budget) {
      return { trip: 'floor_budget', why: `the team has used ${u.tokens.toLocaleString('en')} work tokens, over the budget of ${c.token_budget.toLocaleString('en')}` };
    }
    if (c.cost_budget_usd > 0 && u.cost_usd > c.cost_budget_usd) {
      return { trip: 'cost_budget', why: `the team has used about $${u.cost_usd.toFixed(2)}, over the cost budget of $${c.cost_budget_usd.toFixed(2)}` };
    }
    return null;
  }

  set(cfg: BreakerConfig): BreakerConfig {
    this.cfg = {
      ...cfg,
      token_budget: Math.max(0, cfg.token_budget),
      cost_budget_usd: Math.max(0, cfg.cost_budget_usd),
      velocity_per_min: Math.max(0, cfg.velocity_per_min),
      repeated_tool_limit: cfg.repeated_tool_limit <= 0 ? 0 : Math.max(3, cfg.repeated_tool_limit),
      error_storm_limit: cfg.error_storm_limit <= 0 ? 0 : Math.max(2, cfg.error_storm_limit),
    };
    this.timer ??= setInterval(() => this.beat(), 4000);
    return this.cfg;
  }

  reset() {
    this.baseline = this.today();
    this.levels.clear();
  }

  status(): BreakerStatus {
    const agents: AgentBreaker[] = [...this.sim.agents.values()].map((a) => ({ agent_id: a.id, level: this.levels.get(a.id) ?? 'healthy', work_tokens: this.work(a) }));
    return { used: this.used(), today: this.today(), hold: this.over()?.why ?? null, agents };
  }

  /** What a plan's tasks used: here, what the agents on them used. */
  planSpent(planId: string): Spent {
    const workers = new Set([...this.sim.tasks.values()].filter((t) => t.plan_id === planId).map((t) => t.assignee));
    return this.sum([...this.sim.agents.values()].filter((a) => workers.has(a.id)));
  }

  /** One beat (every 4 s here; 30 s in the core): over budget → one level up for each agent that is running. */
  private beat() {
    const c = this.cfg;
    const over = this.over();
    const ceiling = c.hard_stop ? 3 : 2;
    for (const a of this.sim.agents.values()) {
      if (a.role === 'lead') continue;
      const prev = this.levels.get(a.id) ?? 'healthy';
      if (prev === 'stopped') continue;
      const running = a.status !== 'offline' && a.status !== 'idle' && a.status !== 'done';
      const i = NEXT.indexOf(prev);
      const next = over && running ? NEXT[Math.min(i + 1, ceiling)] : NEXT[Math.max(i - 1, 0)];
      if (next === prev) continue;
      this.levels.set(a.id, next);
      const verb = { steering: 'steered', constrained: 'constrained', stopped: 'stopped', healthy: 'recovered' }[next];
      const up = NEXT.indexOf(next) > i;
      const trip = up ? over : null;
      this.sim.decision(
        'breaker',
        'rule',
        trip ? `Circuit breaker ${verb} ${a.name}: ${trip.why}` : `Circuit breaker: ${a.name} is ${next} again`,
        { level: next, from: prev, action: up ? verb : 'recovered', trip: trip?.trip ?? null, reason: trip?.why ?? null, delivered: true, hard_stop: c.hard_stop },
        { agent: a.id, task: a.task_id },
      );
      if (next === 'stopped') {
        this.sim.emit({ type: 'exited', code: 0 }, { agent: a.id });
        this.sim.patchAgent(a.id, { status: 'offline', activity: null, activity_label: null });
      }
    }
  }
}
