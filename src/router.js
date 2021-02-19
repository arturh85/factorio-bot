import { createRouter, createWebHashHistory } from 'vue-router';
import Dashboard from './pages/Dashboard.vue';

const routes = [
    {
        path: '/',
        name: 'dashboard',
        component: Dashboard,
    },
    {
        path: '/formlayout',
        name: 'formlayout',
        component: () => import('./components/Demo/FormLayoutDemo.vue'),
    },
    {
        path: '/input',
        name: 'input',
        component: () => import('./components/Demo/InputDemo.vue'),
    },
    {
        path: '/floatlabel',
        name: 'floatlabel',
        component: () => import('./components/Demo/FloatLabelDemo.vue'),
    },
    {
        path: '/button',
        name: 'button',
        component: () => import('./components/Demo/ButtonDemo.vue'),
    },
    {
        path: '/table',
        name: 'table',
        component: () => import('./components/Demo/TableDemo.vue'),
    },
    {
        path: '/list',
        name: 'list',
        component: () => import('./components/Demo/ListDemo.vue'),
    },
    {
        path: '/tree',
        name: 'tree',
        component: () => import('./components/Demo/TreeDemo.vue'),
    },
    {
        path: '/panel',
        name: 'panel',
        component: () => import('./components/Demo/PanelsDemo.vue'),
    },
    {
        path: '/overlay',
        name: 'overlay',
        component: () => import('./components/Demo/OverlayDemo.vue'),
    },
    {
        path: '/menu',
        component: () => import('./components/Demo/MenuDemo.vue'),
        children: [
            {
                path: '',
                component: () => import('./components/Demo/menu/PersonalDemo.vue'),
            },
            {
                path: '/menu/seat',
                component: () => import('./components/Demo/menu/SeatDemo.vue'),
            },
            {
                path: '/menu/payment',
                component: () => import('./components/Demo/menu/PaymentDemo.vue'),
            },
            {
                path: '/menu/confirmation',
                component: () => import('./components/Demo/menu/ConfirmationDemo.vue'),
            },
        ],
    },
    {
        path: '/messages',
        name: 'messages',
        component: () => import('./components/Demo/MessagesDemo.vue'),
    },
    {
        path: '/chart',
        name: 'chart',
        component: () => import('./components/Demo/ChartDemo.vue'),
    },
    {
        path: '/misc',
        name: 'misc',
        component: () => import('./components/Demo/MiscDemo.vue'),
    },
    {
        path: '/display',
        name: 'display',
        component: () => import('./components/Demo/utilities/DisplayDemo.vue'),
    },
    {
        path: '/flexbox',
        name: 'flexbox',
        component: () => import('./components/Demo/utilities/FlexBoxDemo.vue'),
    },
    {
        path: '/text',
        name: 'text',
        component: () => import('./components/Demo/utilities/TextDemo.vue'),
    },
    {
        path: '/icons',
        name: 'icons',
        component: () => import('./components/Demo/utilities/Icons.vue'),
    },
    {
        path: '/grid',
        name: 'grid',
        component: () => import('./components/Demo/utilities/GridDemo.vue'),
    },
    {
        path: '/spacing',
        name: 'spacing',
        component: () => import('./components/Demo/utilities/SpacingDemo.vue'),
    },
    {
        path: '/elevation',
        name: 'elevation',
        component: () => import('./components/Demo/utilities/ElevationDemo.vue'),
    },
    {
        path: '/typography',
        name: 'typography',
        component: () => import('./components/Demo/utilities/Typography.vue'),
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
    {
        path: '/documentation',
        name: 'documentation',
        component: () => import('./components/Demo/Documentation.vue'),
    },
];

const router = createRouter({
    history: createWebHashHistory(),
    routes,
});

export default router;
