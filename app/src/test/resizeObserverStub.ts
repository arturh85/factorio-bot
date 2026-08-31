/**
 * jsdom does not implement `ResizeObserver`, which reka-ui's `SliderRoot`
 * calls on mount. Without it, any spec mounting a component containing a
 * Slider dies with `ResizeObserver is not defined` **before a single
 * assertion runs** — so the failure reads as a defect in whatever was being
 * tested rather than as a missing browser API.
 *
 * Imported for its side effect: `import '@/test/resizeObserverStub';`
 *
 * One copy rather than four. This had been pasted identically into
 * `Slider.spec.ts`, `SettingsPage.spec.ts` and `ReplayScrubber.spec.ts`, and a
 * fourth was about to go into `ReplayPanel.spec.ts` — at which point whoever
 * hit it next would paste a fifth.
 *
 * Deliberately not a vitest `setupFiles`: the project's default environment is
 * `node` on purpose, so a global setup would install a DOM shim into specs
 * that have no DOM and do not want one.
 */
class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
}

globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
