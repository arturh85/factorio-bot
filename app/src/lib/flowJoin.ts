/**
 * Joining one flow-graph keyframe to a run's machine samples at a cursor
 * tick.
 *
 * Pure, like every other band's lib: `(flow, samples, tick) -> view model`,
 * no fetch, no store. `flow.jsonl` carries full snapshots rather than
 * deltas, so picking the right keyframe (`flowAt`) is a separate, trivial
 * step from joining it to samples (`flowView`).
 */

import {FlowExport, FlowExportNode, MachineSample, Position, Sample} from '@/api/types';
import {machineStatusAt, positionKey, statusClass, StatusClass} from '@/lib/machineTimeline';
import {RATE_WINDOW_MINUTES} from '@/lib/runRates';
import {TICKS_PER_MINUTE} from '@/lib/tickScale';

// `MachinesSample` is not an exported name in `@/api/types` -- every other
// band that needs it (machineTimeline.ts, runAttribution.ts) derives it
// locally from the `Sample` union the same way. Kept consistent with that
// convention rather than importing a name that does not exist.
type MachinesSample = Extract<Sample, {kind: 'machines'}>;

export interface FlowViewNode {
    id: number;
    position: Position;
    name: string;
    kind: string;
    recipe: string | null;
    minerOre: string | null;
    status: StatusClass;
    /** The item this node's outgoing edges name at the highest modelled
     *  rate, or `null` for a node with no outgoing edge (a sink -- a chest,
     *  say, or the last node on a line). */
    primaryItem: string | null;
    /** Items/minute, modelled: `primaryItem`'s rate summed across every
     *  outgoing edge and lane. `null` exactly when `primaryItem` is. */
    modelPerMinute: number | null;
    /** Items/minute, measured: this node's own machine sample, `primaryItem`'s
     *  production over the trailing window. `null` when there is no machine
     *  sample at this position (a belt, a pipe) or no counter reading there
     *  -- never `0` for "we do not know". */
    measuredPerMinute: number | null;
    /** `modelPerMinute / measuredPerMinute`. `null` whenever either side is
     *  `null` or measured is `0` -- a ratio against nothing said is not a
     *  number. */
    gap: number | null;
}

export interface FlowViewEdge {
    from: number;
    to: number;
    /** Every lane's every item, summed, items/minute -- what stroke width
     *  is drawn from. */
    totalPerMinute: number;
}

export interface FlowView {
    nodes: FlowViewNode[];
    edges: FlowViewEdge[];
}

/** The flow keyframe nearest and at or before `tick`, or `null` when the
 *  run has none yet. `flow` need not be sorted -- this scans it fully,
 *  matching every other cursor-driven "latest at or before" helper on this
 *  page rather than assuming server order. */
export function flowAt(flow: FlowExport[], tick: number): FlowExport | null {
    let latest: FlowExport | null = null;
    for (const f of flow) {
        if (f.tick <= tick && (latest === null || f.tick > latest.tick)) latest = f;
    }
    return latest;
}

function machinesSamples(samples: Sample[]): MachinesSample[] {
    return samples
        .filter((s): s is MachinesSample => s.kind === 'machines')
        .sort((a, b) => a.tick - b.tick);
}

function byPosition(machines: Record<string, MachineSample>): Map<string, MachineSample> {
    const out = new Map<string, MachineSample>();
    for (const m of Object.values(machines)) out.set(positionKey(m.position), m);
    return out;
}

/**
 * Each machine's own production delta over `(lo, hi]`, by position key.
 *
 * Deliberately NOT `runAttribution.ts`'s `machineProduction` -- that
 * function aggregates by item and entity *name* for the roster-fed/factory
 * verdict, collapsing every furnace of the same kind into one bucket. This
 * needs one specific machine's own count, to join to one specific flow
 * node.
 *
 * `baseAt` defaults to the EARLIEST `machines` sample, not to "no data" --
 * a run's first samples land after tick 0, so a cursor inside the first
 * window has nothing at-or-before `lo` at all. Treating that as a zero
 * baseline would price a machine's entire lifetime output as if it were
 * made inside one trailing window; falling back to the oldest sample we do
 * have measures "since we started watching" instead, which is what a
 * reader staring at the first few minutes of a run actually wants.
 */
function machineDeltaAt(samples: Sample[], lo: number, hi: number): Map<string, number> {
    const rows = machinesSamples(samples);
    const out = new Map<string, number>();
    if (rows.length === 0) return out;
    let baseAt: MachinesSample = rows[0];
    let endAt: MachinesSample | null = null;
    for (const s of rows) {
        if (s.tick <= lo) baseAt = s;
        if (s.tick <= hi) endAt = s;
        else break;
    }
    if (endAt === null) return out;
    const base = byPosition(baseAt.machines);
    for (const m of Object.values(endAt.machines)) {
        if (m.produced === null || m.produced === undefined) continue;
        const key = positionKey(m.position);
        const before = base.get(key)?.produced ?? 0;
        const delta = m.produced - before;
        if (delta > 0) out.set(key, (out.get(key) ?? 0) + delta);
    }
    return out;
}

function outgoingRatesByNode(flow: FlowExport): Map<number, Map<string, number>> {
    const out = new Map<number, Map<string, number>>();
    for (const edge of flow.edges) {
        const totals = out.get(edge.from) ?? new Map<string, number>();
        for (const lane of edge.lanes) {
            for (const rate of lane) {
                totals.set(rate.item, (totals.get(rate.item) ?? 0) + rate.per_second);
            }
        }
        out.set(edge.from, totals);
    }
    return out;
}

function primaryOf(outgoing: Map<string, number> | undefined): {item: string | null; perSecond: number} {
    if (outgoing === undefined) return {item: null, perSecond: 0};
    let item: string | null = null;
    let perSecond = 0;
    for (const [candidate, rate] of outgoing) {
        if (item === null || rate > perSecond) {
            item = candidate;
            perSecond = rate;
        }
    }
    return {item, perSecond};
}

function viewNode(
    node: FlowExportNode,
    outgoing: Map<string, number> | undefined,
    statuses: Map<string, string | null>,
    produced: Map<string, number>,
    windowMinutes: number
): FlowViewNode {
    const key = positionKey(node.position);
    const status = statusClass(statuses.get(key) ?? null);
    const {item: primaryItem, perSecond} = primaryOf(outgoing);
    const modelPerMinute = primaryItem === null ? null : perSecond * 60;
    const madeThisWindow = produced.get(key);
    const measuredPerMinute = madeThisWindow === undefined ? null : madeThisWindow / windowMinutes;
    const gap =
        modelPerMinute !== null && measuredPerMinute !== null && measuredPerMinute > 0
            ? modelPerMinute / measuredPerMinute
            : null;
    return {
        id: node.id, position: node.position, name: node.name, kind: node.kind,
        recipe: node.recipe, minerOre: node.miner_ore, status,
        primaryItem, modelPerMinute, measuredPerMinute, gap
    };
}

/** Joins one flow keyframe to `samples` at `cursorTick`, over a trailing
 *  window of `windowMinutes` (default: the same `RATE_WINDOW_MINUTES` every
 *  other band on this page uses). */
export function flowView(
    flow: FlowExport,
    samples: Sample[],
    cursorTick: number,
    windowMinutes: number = RATE_WINDOW_MINUTES
): FlowView {
    const windowTicks = windowMinutes * TICKS_PER_MINUTE;
    const lo = Math.max(0, cursorTick - windowTicks);
    const statuses = machineStatusAt(samples, cursorTick);
    const produced = machineDeltaAt(samples, lo, cursorTick);
    const outgoing = outgoingRatesByNode(flow);

    const nodes = flow.nodes.map((n) => viewNode(n, outgoing.get(n.id), statuses, produced, windowMinutes));

    const edges: FlowViewEdge[] = flow.edges.map((e) => ({
        from: e.from,
        to: e.to,
        totalPerMinute: e.lanes.flat().reduce((sum, r) => sum + r.per_second * 60, 0)
    }));

    return {nodes, edges};
}
