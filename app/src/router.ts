import { createRouter, createWebHashHistory } from 'vue-router';

const routes = [
    {
        path: '/',
        name: 'dashboard',
        component:  () => import('./pages/Dashboard.vue')
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
        path: '/empty',
        name: 'empty',
        component: () => import('./pages/EmptyPage.vue')
    },
    {
        path: '/factorioMods',
        name: 'factorioMods',
        component: () => import('./pages/EmptyPage.vue')
    },
    {
        path: '/restApiDocss',
        name: 'restApiDocss',
        component: () => import('./pages/EmptyPage.vue')
    },
    {
        path: '/luaApiDocss',
        name: 'luaApiDocss',
        component: () => import('./pages/EmptyPage.vue')
    },
    {
        path: '/workspace',
        name: 'workspace',
        component: () => import('./pages/EmptyPage.vue')
    },
    {
        path: '/instances',
        name: 'instances',
        component: () => import('./pages/GameInstances.vue')
    },
    {
        path: '/taskGraph',
        name: 'taskGraph',
        component: () => import('./pages/TaskGraphPage.vue')
    }
];

const router = createRouter({
    history: createWebHashHistory(),
    routes
});

export default router;
