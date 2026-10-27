// Mock circuit breaker for `pnpm dev` and the site demo: the same steer → constrain → stop ladder as
// the core's, driven by the one signal the scripted story really has, work tokens. Set a token
// budget below what the team has used (Settings → Autonomy & Budgets) and the working agents are
// steered, then constrained, and with Hard stop stopped, each step a decision on their timeline.
import type { AgentBreaker } from '../gen/AgentBreaker';
import type { BreakerConfig } from '../gen/BreakerConfig';
import type { BreakerLevel } from '../gen/BreakerLevel';
import type { BreakerStatus } from '../gen/BreakerStatus';
import type { Sim } from './sim';

const DEFAULT: BreakerConfig = { enabled: true, token_budget: 0, velocity_per_min: 60_000, repeated_tool_limit: 8, error_storm_limit: 5, hard_stop: false };
const NEXT: BreakerLevel[] = ['healthy', 'steering', 'constrained', 'stopped'];

export class MockBreaker {
  cfg: BreakerConfig = { ...DEFAULT };
  private levels = new Map<string, BreakerLevel>();
  private baseline = 0;
  private timer: ReturnType<typeof setInterval> | null = null;

  constructor(private sim: Sim) {}

  private work(a: { usage: { input_tokens: number; output_tokens: number; cache_write_tokens: number } }) {
    return a.usage.input_tokens + a.usage.output_tokens + a.usage.cache_write_tokens;
  }
  private floor() {
    return Math.max(0, [...this.sim.agents.values()].reduce((n, a) => n + this.work(a), 0) - this.baseline);
  }

  set(cfg: BreakerConfig): BreakerConfig {
    this.cfg = {
      ...cfg,
      token_budget: Math.max(0, cfg.token_budget),
      velocity_per_min: Math.max(0, cfg.velocity_per_min),
      repeated_tool_limit: cfg.repeated_tool_limit <= 0 ? 0 : Math.max(3, cfg.repeated_tool_limit),
      error_storm_limit: cfg.error_storm_limit <= 0 ? 0 : Math.max(2, cfg.error_storm_limit),
    };
    this.timer ??= setInterval(() => this.beat(), 4000);
    return this.cfg;
  }

  reset() {
    this.baseline = this.baseline + this.floor();
    this.levels.clear();
  }

  status(): BreakerStatus {
    const agents: AgentBreaker[] = [...this.sim.agents.values()].map((a) => ({ agent_id: a.id, level: this.levels.get(a.id) ?? 'healthy', work_tokens: this.work(a) }));
    return { floor_tokens: this.floor(), agents };
  }

  /** One beat (every 4 s here; 30 s in the core): over budget → one level up for each agent that is running. */
  private beat() {
    const c = this.cfg;
    const over = c.enabled && c.token_budget > 0 && this.floor() > c.token_budget;
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
      const why = `the team has used ${this.floor().toLocaleString('en')} work tokens, over the budget of ${c.token_budget.toLocaleString('en')}`;
      const up = NEXT.indexOf(next) > i;
      this.sim.decision(
        'breaker',
        'rule',
        up ? `Circuit breaker ${verb} ${a.name}: ${why}` : `Circuit breaker: ${a.name} is ${next} again`,
        { level: next, from: prev, action: up ? verb : 'recovered', trip: up ? 'floor_budget' : null, reason: up ? why : null, delivered: true, hard_stop: c.hard_stop },
        { agent: a.id, task: a.task_id },
      );
      if (next === 'stopped') {
        this.sim.emit({ type: 'exited', code: 0 }, { agent: a.id });
        this.sim.patchAgent(a.id, { status: 'offline', activity: null, activity_label: null });
      }
    }
  }
}
