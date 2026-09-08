/** The first sentence a reader sees: the analysis tool's headline shape. */
import {Event, Sample, Split} from '@/api/types';
import {attributeInterval} from './runAttribution';
import {ForceSample, markAt} from './runRates';
import {formatGameTime, markTicks} from './tickScale';

export function headline(input: {samples: Sample[]; events: Event[]; splits: Split[]; lo: number; hi: number; items: string[]}): string {
    const {samples, events, splits, lo, hi, items} = input;
    const scale = {from: lo, to: hi};
    const parts: string[] = [];
    const mark = markTicks(scale)[0];
    if (mark !== undefined && items.length > 0) {
        const firstGen = samples.filter((s): s is ForceSample => s.kind === 'force').sort((a, b) => a.tick - b.tick).find((s) => s.power.generated_kw > 0) ?? null;
        const rates = items.slice(0, 2).map((item, i) => {
            const m = markAt(samples, lo, hi, 5, 0, item);
            const a = attributeInterval(samples, events, lo, mark, item);
            const rate = m.status === 'ok' && m.rateWindow !== null ? `${m.rateWindow.toFixed(0)}/min` : m.status.replace('_', ' ');
            const gen = i === 0 && firstGen !== null && firstGen.tick > lo ? `; no generator until ${formatGameTime(scale, firstGen.tick)}` : '';
            return `${item} ${rate} at ${formatGameTime(scale, mark)} (${a.verdict}${gen})`;
        });
        parts.push(`rates: ${rates.join(' · ')}`);
    }
    const last = [...splits].reverse().find((s) => s.ended_tick !== null);
    if (last) parts.push(`milestone ${last.index} ${last.goal} ${last.outcome} at ${formatGameTime(scale, last.ended_tick as number)}`);
    return parts.length === 0 ? 'no marks reached and no milestones closed' : parts.join(' | ');
}
