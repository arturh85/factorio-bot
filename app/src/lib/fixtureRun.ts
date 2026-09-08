/**
 * One real archived run, for tests that must agree with `tools/run_analysis.py`.
 *
 * Node-only (reads the filesystem); never imported by app code. The golden
 * `rates.json` is what pins `runAttribution.ts` to the Python rule.
 */
import {readFileSync} from 'node:fs';
import {join} from 'node:path';
import {Event, Lane, Sample} from '@/api/types';

export type Verdict = 'roster-fed' | 'factory' | 'hand-made' | 'mixed' | 'unclear' | 'no output';

export interface GoldenItem {
    cumulative: number | null;
    made_in_interval: number | null;
    rate_interval: number | null;
    rate_window: number | null;
    machine_made?: number;
    roster_made?: number;
    source?: string;
    verdict: Verdict;
    why: string;
}

export interface GoldenMark {
    minute: number;
    label: string;
    tick: number;
    is_end: boolean;
    status: 'ok' | 'run_ended' | 'samples_end' | 'no_sample';
    sample_tick: number | null;
    lag_ticks: number | null;
    items: Record<string, GoldenItem>;
    attribution?: {from_tick: number; roster: {feed_actions: number}};
}

export interface RatesGolden {
    origin_tick: number;
    end_tick: number;
    window_minutes: number;
    items: string[];
    marks: GoldenMark[];
    first_generation_tick: number | null;
}

export interface FixtureRun {
    events: Event[];
    samples: Sample[];
    lanes: Lane[];
    rates: RatesGolden;
    lo: number;
    hi: number;
}

const DIR = join(__dirname, '__fixtures__', 'run-1788696619-00325');

function jsonl<T>(file: string): T[] {
    return readFileSync(join(DIR, file), 'utf8')
        .split('\n')
        .filter((line) => line.trim().length > 0)
        .map((line) => JSON.parse(line) as T);
}

export function loadFixtureRun(): FixtureRun {
    const events = jsonl<Event>('events.jsonl');
    const samples = jsonl<Sample>('samples.jsonl');
    const lanes = (JSON.parse(readFileSync(join(DIR, 'lanes.json'), 'utf8')) as {lanes: Lane[]}).lanes;
    const rates = JSON.parse(readFileSync(join(DIR, 'rates.json'), 'utf8')) as RatesGolden;
    return {events, samples, lanes, rates, lo: rates.origin_tick, hi: rates.end_tick};
}
