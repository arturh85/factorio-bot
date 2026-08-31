import {createRouter, createWebHashHistory} from 'vue-router';

// Six further routes existed until this commit: /empty, /factorioMods,
// /restApiDocss, /luaApiDocss and /workspace all rendered the same stock
// "Empty Page" placeholder, and /instances rendered a static card with no data
// binding. None was reachable from the menu. They are deleted rather than
// restyled; see docs/superpowers/plans/2026-08-31-shadcn-ui-redesign.md.
//
// /map is new, not restored: the commented-out 'Map' menu entry App.vue used
// to carry pointed at the deleted /workspace above, which was an unbuilt
// placeholder, not this route. See .superpowers/sdd/map-view/brief.md.
const routes = [
    {
        path: '/',
        name: 'dashboard',
        component: () => import('./pages/Dashboard.vue')
    },
    {
        path: '/settings',
        name: 'settings',
        component: () => import('./pages/SettingsPage.vue')
    },
    {
        path: '/script',
        name: 'script',
        component: () => import('./pages/ScriptPage.vue')
    },
    {
        path: '/rcon',
        name: 'rcon',
        component: () => import('./pages/RconPage.vue')
    },
    {
        path: '/tasks',
        name: 'tasks',
        component: () => import('./pages/TasksPage.vue')
    },
    {
        path: '/map',
        name: 'map',
        component: () => import('./pages/MapPage.vue')
    }
];

const router = createRouter({
    history: createWebHashHistory(),
    routes
});

export default router;
