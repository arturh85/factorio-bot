import { createRouter, createWebHashHistory } from 'vue-router';
import Dashboard from './pages/Dashboard.vue';

const routes = [
    {
        path: '/',
        name: 'dashboard',
        component: Dashboard,
    },
    {
        path: '/empty',
        name: 'empty',
        component: () => import('./pages/EmptyPage.vue'),
    },
    {
        path: '/instances',
        name: 'instances',
        component: () => import('./pages/GameInstances.vue'),
    },
];

const router = createRouter({
    history: createWebHashHistory(),
    routes,
});

export default router;
