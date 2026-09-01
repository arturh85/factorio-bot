// @vitest-environment jsdom
import {describe, expect, it} from 'vitest';
import router from './router';

const paths = () => router.getRoutes().map(route => route.path).sort();

describe('router', () => {
    it('exposes exactly the seven routes the menu links to, plus the analysis view reached from a run', () => {
        expect(paths()).toEqual([
            '/',
            '/map',
            '/rcon',
            '/runs',
            '/runs/:id/analysis',
            '/script',
            '/settings',
            '/tasks'
        ]);
    });

    it('does not resolve the deleted placeholder routes', () => {
        for (const path of ['/empty', '/factorioMods', '/restApiDocss', '/luaApiDocss', '/workspace', '/instances']) {
            expect(router.resolve(path).matched).toEqual([]);
        }
    });
});
