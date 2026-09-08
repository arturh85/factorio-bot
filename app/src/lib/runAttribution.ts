/**
 * WHO MADE IT. A port of `attribute_output` / `attribute_from_counters` /
 * `machine_production` / `feeding_dispatches` in `tools/run_analysis.py`,
 * pinned to that tool's JSON by `runAttribution.spec.ts`.
 *
 * `production.made` counts what a MACHINE produced, and a stone furnace a bot
 * hand-loaded is a machine, so a rising curve is not evidence of a working
 * factory. The split is arithmetic: what the machines' own counters say they
 * made against what the force's statistics say was made; the remainder is
 * hand work. The count of feeding dispatches -- not their ticks, five of the
 * six settle in the tick they dispatch -- decides roster-fed against factory.
 *
 * **The counter path is a port; the inference path is a REDUCTION.**
 * `fromCounters` follows `attribute_from_counters` question for question,
 * and the golden tests hold it there. `byInference` keeps only the first of
 * `_attribute_by_inference`'s four branches (`any_generation`) and can
 * therefore be more confident than the tool on the same interval -- see its
 * own doc for the case. Anything it returns carries `source: 'inference'`,
 * which the band draws dashed and words as "(inferred)", so the weaker claim
 * is never presented as the stronger one.
 */
import {Event, MachineSample, Sample} from '@/api/types';
import {madeAt} from './runRates';
import {TICKS_PER_MINUTE} from './tickScale';

export const FEEDING_VERBS = ['insert', 'stock', 'charge', 'fuel', 'take', 'mine'];

export type Verdict = 'roster-fed' | 'factory' | 'hand-made' | 'mixed' | 'unclear' | 'no output';

export interface MachineProduction {
    /**
     * False for a run archived before the counters existed; never "made
     * nothing". Also false for an interval with NO machines at all -- an
     * empty set carries no evidence a counter was ever read, so it routes
     * to the inference fallback exactly as a pre-counter archive does
     * (matches `machine_production`'s `any(...)` over `end_rows.values()`
     * in `tools/run_analysis.py`, which is vacuously false on an empty dict).
     */
    available: boolean;
    byItem: Record<string, number>;
    byName: Record<string, Record<string, number>>;
    total: number;
    unattributed: number;
    shared: string[];
}

export interface Attribution {
    verdict: Verdict;
    machineMade: number;
    rosterMade: number;
    feeds: number;
    delta: number;
    source: 'counters' | 'inference';
    why: string;
}

type MachinesSample = Extract<Sample, {kind: 'machines'}>;

export function verbOf(action: string): string {
    return action.trim().split(/\s+/)[0] ?? '';
}

/**
 * Feeding-verb dispatches in `(lo, hi]` -- the count, deliberately.
 *
 * A port of the standalone `feeding_dispatches` (`tools/run_analysis.py`
 * ~2326), which reads `action_dispatched` straight and needs no roster and
 * no join. **That is not the same counter the tool reports beside a mark**:
 * the `feed_actions` in its attribution block comes from `interval_activity`
 * (~2126), which walks `dispatches` PER BOT, so a dispatch carrying no `bot`
 * field is invisible to it and counted here. Identical on the archived
 * fixture -- every feeding dispatch there names a bot -- and
 * `runAttribution.spec.ts` compares this against the tool's number on that
 * basis, so a run whose feeding dispatches lack a bot would diverge and the
 * spec would be right to.
 */
export function feedingDispatches(events: Event[], lo: number, hi: number): number {
    let n = 0;
    for (const e of events) {
        if (e.kind !== 'action_dispatched') continue;
        if (!(lo < e.tick && e.tick <= hi)) continue;
        if (FEEDING_VERBS.includes(verbOf(e.action))) n += 1;
    }
    return n;
}

function itemOf(m: MachineSample): string | null {
    return m.recipe ?? m.mining ?? null;
}

/**
 * The `machines` samples in tick order, memoised on the array's identity --
 * the same shape, and the same reason, as `runRates.forceSamples`:
 * `attributionIntervals` calls this once per minute of the run and the answer
 * cannot change while the array does not. See that function's doc for why
 * identity is the key and what it cannot survive.
 */
const MACHINE_ORDER = new WeakMap<Sample[], MachinesSample[]>();

function machinesSamples(samples: Sample[]): MachinesSample[] {
    const cached = MACHINE_ORDER.get(samples);
    if (cached !== undefined) return cached;
    const ordered = samples
        .filter((s): s is MachinesSample => s.kind === 'machines')
        .sort((a, b) => a.tick - b.tick);
    MACHINE_ORDER.set(samples, ordered);
    return ordered;
}

/** What each machine itself produced in `(lo, hi]`, from its own counter. */
export function machineProduction(samples: Sample[], lo: number, hi: number): MachineProduction {
    const rows = machinesSamples(samples);
    let baseAt: MachinesSample | null = null;
    let endAt: MachinesSample | null = null;
    // The last item each machine was ever seen making, up to `hi`: an idle
    // stone furnace reports no recipe, and reading only the final row would
    // file every plate it smelted under "unattributed".
    const named = new Map<string, string>();
    for (const s of rows) {
        if (s.tick <= lo) baseAt = s;
        if (s.tick <= hi) endAt = s;
        else continue;
        for (const [key, m] of Object.entries(s.machines)) {
            const item = itemOf(m);
            if (item !== null) named.set(key, item);
        }
    }
    const baseRows = baseAt?.machines ?? {};
    const endRows = endAt?.machines ?? {};
    const out: MachineProduction = {
        available: Object.values(endRows).some((m) => m.produced_source !== null && m.produced_source !== undefined),
        byItem: {}, byName: {}, total: 0, unattributed: 0, shared: []
    };
    for (const [key, m] of Object.entries(endRows)) {
        const source = m.produced_source;
        if (source === 'not-a-producer' || source === 'unavailable') continue;
        if (source === null || source === undefined || m.produced === null) continue;
        const before = baseRows[key]?.produced ?? 0;
        const delta = m.produced - before;
        if (delta <= 0) continue;
        out.total += delta;
        const item = itemOf(m) ?? named.get(key) ?? null;
        if (item === null) {
            out.unattributed += delta;
            continue;
        }
        out.byItem[item] = (out.byItem[item] ?? 0) + delta;
        (out.byName[item] ??= {})[m.name] = (out.byName[item]?.[m.name] ?? 0) + delta;
        if (m.produced_shared) out.shared.push(m.name);
    }
    return out;
}

function fromCounters(delta: number, produced: MachineProduction, item: string, feeds: number): Attribution {
    const machineMade = produced.byItem[item] ?? 0;
    const names = Object.entries(produced.byName[item] ?? {}).map(([n, c]) => `${n}x${c}`).join(', ');
    const fed = `; the roster ran ${feeds} feeding action(s)`;
    const common = {machineMade, rosterMade: Math.max(0, delta - machineMade), feeds, delta, source: 'counters' as const};
    if (machineMade > delta * 1.05 + 1) {
        const shared = [...new Set(produced.shared)].sort().join(', ') || 'none flagged';
        return {...common, verdict: 'unclear', why: `the machines' own counters say ${machineMade} while the force's statistics say ${delta} was made -- they cannot both be right. Drills sharing a resource tile double-count and are flagged: ${shared}`};
    }
    if (machineMade === 0) {
        return {...common, verdict: 'hand-made', why: `no machine produced any of the ${delta} made in this interval -- every one of them came out of the roster's own hands (hand crafting and hand mining pass through no machine)${fed}`};
    }
    const share = machineMade / delta;
    if (share >= 0.95) {
        if (feeds > 0) {
            return {...common, verdict: 'roster-fed', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the roster ran ${feeds} feeding action(s), so the machines produced it and the bots carried what went in`};
        }
        return {...common, verdict: 'factory', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the roster fed nothing in this interval`};
    }
    return {...common, verdict: 'mixed', why: `machines made ${machineMade} of the ${delta} (${Math.round(share * 100)}%) -- ${names} -- and the remaining ${delta - machineMade} was hand-made${fed}`};
}

/**
 * The fallback for a run archived before the per-machine counters existed.
 * It INFERS, and says so in `source`: feeding dispatches plus any generation
 * decide, and `unclear` is said freely.
 *
 * **A REDUCTION of `_attribute_by_inference` (`tools/run_analysis.py`
 * ~3246-3320), not a port of it.** The Python walks four questions in order
 * -- any generation, any consumption, then whether an ELECTRIC producer was
 * working and which machines drew the power -- because power drawn is not
 * evidence about items until the machines that made them are named: a lab at
 * 120 kW says nothing about plates a hand-loaded furnace smelted. This keeps
 * only the first of those, `any_generation`, because it is all the browser
 * reads today.
 *
 * So the two can differ, and the case is nameable: with `feeds === 0` and
 * generation present, this says `factory`, while the Python says `unclear`
 * whenever no electric producer worked in the interval -- the output came out
 * of burner machines loaded earlier, and the power went somewhere else. Any
 * verdict from here is the weaker claim; that is exactly why it is marked
 * `inference` and drawn dashed.
 */
function byInference(delta: number, feeds: number, samples: Sample[], lo: number, hi: number): Attribution {
    const anyGeneration = samples.some((s) => s.kind === 'force' && s.tick > lo && s.tick <= hi && s.power.generated_kw > 0);
    const common = {machineMade: 0, rosterMade: 0, feeds, delta, source: 'inference' as const};
    if (feeds > 0 && !anyGeneration) return {...common, verdict: 'roster-fed', why: `the roster ran ${feeds} feeding action(s) and nothing generated electricity, so the bots carried what the machines ate (inferred: this run has no machine counters)`};
    if (feeds === 0 && anyGeneration) return {...common, verdict: 'factory', why: 'electricity was drawn and the roster fed nothing in this interval (inferred: this run has no machine counters)'};
    return {...common, verdict: 'unclear', why: `${feeds} feeding action(s) and ${anyGeneration ? 'some' : 'no'} generation -- the record cannot separate the roster from the factory here (inferred)`};
}

/** Who earned `item`'s output over `(lo, hi]`. */
export function attributeInterval(samples: Sample[], events: Event[], lo: number, hi: number, item: string): Attribution {
    const c = madeAt(samples, hi, item, lo);
    const cPrev = madeAt(samples, lo, item, lo) ?? 0;
    const delta = c === null ? 0 : c - cPrev;
    if (delta <= 0) {
        return {verdict: 'no output', machineMade: 0, rosterMade: 0, feeds: 0, delta: 0, source: 'counters', why: 'nothing made in this interval'};
    }
    const feeds = feedingDispatches(events, lo, hi);
    const produced = machineProduction(samples, lo, hi);
    if (produced.available) return fromCounters(delta, produced, item, feeds);
    return byInference(delta, feeds, samples, lo, hi);
}

/** The verdict per fixed-width interval across `[lo, hi]`; the last interval may be short. */
export function attributionIntervals(
    samples: Sample[], events: Event[], lo: number, hi: number, item: string, stepTicks = TICKS_PER_MINUTE
): (Attribution & {from: number; to: number})[] {
    const out: (Attribution & {from: number; to: number})[] = [];
    for (let from = lo; from < hi; from += stepTicks) {
        const to = Math.min(hi, from + stepTicks);
        out.push({from, to, ...attributeInterval(samples, events, from, to, item)});
    }
    return out;
}
