// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import {mount} from '@vue/test-utils';
import RunHeadline from './RunHeadline.vue';

const summary = {run_id: 'run-1', finished: true, started_unix: 1, finished_unix: 2, outcome: 'done', elapsed_ticks: 10, events: 1, splits: 1, samples: 5, map: 1, samples_lag_ticks: 0};
const provenance = {schema: 1, run_id: 'run-1', started_unix: 1, started_tick: 0, seed: '31337', map_exchange_string: null, map: {digest: 'c161fa3f437221d0', tiles: {}}, factorio: '2.1.17', mods: {base: '2.1.17', BotBridge: '0.0.1'}, git: {commit: '492e513a261bde8f4c433ecdd4e749918f9a6160', dirty: true, source: 'working-tree-at-run-start'}, profile: 'release', roster_requested: [1, 2, 3, 4], workspace: null, resumed_from: null, bot_mode: 'clients', game_speed: 1, peaceful: null};
const base = {summary, lagTicks: 0, headline: 'rates: …', roster: [1, 2, 3, 4], savepoints: [], replayCounts: null};

describe('RunHeadline', () => {
    it('fills the chips from provenance and marks a dirty commit', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('present');
        expect(w.get('[data-chip="seed"]').text()).toContain('31337');
        expect(w.get('[data-chip="commit"]').text()).toContain('492e513a dirty');
        expect(w.get('[data-chip="mods"]').text()).toContain('2 mods');
        expect(w.get('[data-chip="mods"]').attributes('title')).toContain('BotBridge 0.0.1');
        expect(w.get('[data-chip="map"]').text()).toContain('c161fa3f437221d0');
    });
    it('a null field inside a present provenance is "not captured" for that chip only', () => {
        const w = mount(RunHeadline, {props: {...base, provenance: {...provenance, seed: null}, provenanceError: null}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="mode"]').attributes('data-state')).toBe('present');
    });
    it('a missing provenance file marks every chip absent and says why', () => {
        const w = mount(RunHeadline, {props: {...base, provenance: null, provenanceError: 'provenance unavailable — this server does not provide /provenance'}});
        expect(w.get('[data-chip="seed"]').attributes('data-state')).toBe('absent');
        expect(w.get('[data-chip="provenance"]').text()).toContain('does not provide /provenance');
    });
    it('shows the plan counts and flags an abandoned tail', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null, replayCounts: {steps: 176, abandoned: 48, lost: 0, failed: 1, pending: 0, believed: 52}}});
        const plan = w.get('[data-chip="plan"]');
        expect(plan.text()).toContain('176 steps');
        expect(plan.text()).toContain('48 abandoned');
        expect(plan.text()).not.toContain('0 lost');
        expect(plan.attributes('data-state')).toBe('truncated');
    });
    it('offers one resume command per savepoint', () => {
        const w = mount(RunHeadline, {props: {...base, provenance, provenanceError: null, savepoints: [{schema: 1, run_id: 'run-1', milestone_index: 1, tick: 300, created_unix: 1, bytes: 10, file: 'milestone-1.zip', mods: null}]}});
        expect(w.get('[data-chip="resume"] code').text()).toBe('--resume-from run-1:1');
    });
});
