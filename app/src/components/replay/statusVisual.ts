import {CheckCircle2, CircleHelp, Loader2, XCircle} from '@lucide/vue';
import {ReplayStatus} from '@/api/replay';

/**
 * `Failed` and `Lost` must never share a look: one is a verdict, the other is
 * the absence of one. Icon, colour classes and accessible label all differ,
 * on purpose, for every status -- this table is the single place that
 * decides how a status reads, so nothing downstream can accidentally align
 * two statuses that must stay visually apart.
 */
export const STATUS_VISUAL: Record<ReplayStatus, {icon: typeof CheckCircle2 | null; classes: string; label: string}> = {
    Pending: {icon: null, classes: '', label: 'pending'},
    Running: {
        icon: Loader2,
        classes: 'border border-brand bg-brand/20 text-brand-dark',
        label: 'running -- outcome not yet known'
    },
    Success: {
        icon: CheckCircle2,
        classes: 'bg-success text-white',
        label: 'success -- the ticks were measured'
    },
    Failed: {
        icon: XCircle,
        classes: 'bg-danger text-white',
        label: 'failed -- a verdict arrived and it was bad'
    },
    Lost: {
        icon: CircleHelp,
        // Amber, hatched, question-marked: an outcome that DID happen (this
        // run stopped watching) and is meant to draw the eye like `Failed`
        // does -- but never the danger-red that would say a bad verdict
        // arrived, because none did. Deliberately shares no colour token
        // with `EvidenceMark`'s quiet ink-muted styling: `Lost` is an
        // outcome and must not read as the same kind of thing as a caveat
        // on a routine row.
        classes:
            'border-2 border-dashed border-warn text-ink ' +
            'bg-[repeating-linear-gradient(45deg,color-mix(in_srgb,var(--color-warn)_40%,transparent)_0px,' +
            'color-mix(in_srgb,var(--color-warn)_40%,transparent)_3px,transparent_3px,transparent_7px)]',
        label: 'lost -- this run will never learn the outcome'
    }
};

/**
 * The legend's rows, derived from the table above rather than restated
 * beside it.
 *
 * A restated legend is a mirror: add a status and the legend silently stops
 * describing the view. Deriving it means a new status appears in the key for
 * free, and a changed colour changes the swatch, because there is only one
 * place either is written down.
 *
 * `Pending` is omitted deliberately -- it has no visual of its own (a pending
 * step draws no observed bar at all), so a swatch for it would be an empty
 * box next to the words "pending", which explains nothing.
 */
export const LEGEND_ENTRIES = (Object.entries(STATUS_VISUAL) as [ReplayStatus, (typeof STATUS_VISUAL)[ReplayStatus]][])
    .filter(([status]) => status !== 'Pending');
