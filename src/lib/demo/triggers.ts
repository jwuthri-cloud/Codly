// The mock backend's Triggers: the same normalising as crates/codly-core/src/triggers.rs, seeded
// with the hourly standup so the demo panel shows a last-fired time, a next run and a log.
import type { ContextRule } from '../gen/ContextRule';
import type { Schedule } from '../gen/Schedule';
import type { TriggersConfig } from '../gen/TriggersConfig';

const MIN = 60_000;
const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, Math.round(n) || lo));

export function seedTriggers(now = Date.now()): TriggersConfig {
  return {
    schedules: [
      { id: 'sch-standup', name: 'Hourly ops standup', prompt: 'Review every agent: who is stuck, who is idle, what is blocked on me? Reply in five lines.', every_minutes: 60, enabled: true, created_at: now - 5 * 60 * MIN, last_fired_at: now - 25 * MIN },
      { id: 'sch-triage', name: 'Morning triage', prompt: 'Look at open branches and failing checks. Propose a plan for anything that is red.', every_minutes: 1440, enabled: false, created_at: now - 2 * 1440 * MIN, last_fired_at: null },
    ],
    context: { compact: { enabled: true, every_hours: 2, min_percent: 60 }, clear: { enabled: false, every_hours: 8, min_percent: 90 }, compact_focus: '' },
    network: { mode: 'off', advanced: { confirm_handoffs: true, peers: [] } },
    state: {
      context: {},
      log: [
        { ts: now - 25 * MIN, kind: 'schedule', label: 'Hourly ops standup', detail: 'sent to the Lead', ok: true },
        { ts: now - 70 * MIN, kind: 'compact', label: 'Bo', detail: 'asked it to compact at 71%', ok: true },
        { ts: now - 85 * MIN, kind: 'schedule', label: 'Hourly ops standup', detail: 'sent to the Lead', ok: true },
      ],
    },
  };
}

/** What the core does on `set_triggers`: ids filled in, numbers clamped, its own fields kept. */
export function saveTriggers(old: TriggersConfig, next: TriggersConfig, now = Date.now()): TriggersConfig {
  const schedules = next.schedules.map((s): Schedule => {
    const prev = old.schedules.find((o) => o.id === s.id);
    const prompt = s.prompt.trim();
    const name = s.name.split(/\s+/).filter(Boolean).join(' ') || prompt.slice(0, 40);
    const every = clamp(s.every_minutes, 5, 7 * 1440);
    const overdue = prev?.last_fired_at != null && prev.last_fired_at + prev.every_minutes * MIN <= now;
    const fired = prev ? (prev.every_minutes !== every && overdue ? now : prev.last_fired_at) : null;
    return { ...s, id: s.id || `sch-${Math.random().toString(36).slice(2, 8)}`, name, prompt, every_minutes: every, created_at: prev?.created_at ?? now, last_fired_at: fired };
  });
  const rule = (r: ContextRule): ContextRule => ({ ...r, every_hours: clamp(r.every_hours, 1, 168), min_percent: clamp(r.min_percent, 0, 100) });
  return {
    schedules,
    context: { compact: rule(next.context.compact), clear: rule(next.context.clear), compact_focus: next.context.compact_focus.trim().slice(0, 300) },
    network: { ...next.network, advanced: { ...next.network.advanced, peers: next.network.advanced.peers.map((p) => p.trim()).filter(Boolean) } },
    state: old.state,
  };
}
