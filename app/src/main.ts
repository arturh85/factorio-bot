import { createApp } from 'vue';
import router from './router';
import {createPinia} from 'pinia';
import PrimeVue from 'primevue/config';
import Lara from '@primeuix/themes/lara';
import Tooltip from 'primevue/tooltip';
import Ripple from 'primevue/ripple';
import {useToast} from './composables/useToast';

import './assets/tailwind.css';
import 'primeicons/primeicons.css';
import './assets/layout/layout.scss';

import App from './App.vue';

// Vue Router 5 deprecates the next() callback; returning undefined continues
// the navigation, and next() is removed entirely in Router 6.
router.beforeEach(() => {
    // Messages are about the page the user is leaving. Clearing here rather
    // than in a component hook also covers navigations that unmount nothing.
    useToast().clear();
    window.scrollTo(0, 0);
});

const app = createApp(App);
const store = createPinia()

// PrimeVue 4 replaced the shipped theme stylesheets with a runtime theme
// service, so app.use(PrimeVue, ...) is mandatory rather than optional and
// installs $primevue itself. Lara is the closest preset to the saga-blue theme
// this app used under PrimeVue 3; swapping it for Aura, Nora or Material is a
// one-line change here.
app.use(PrimeVue, {
    ripple: true,
    theme: {
        preset: Lara,
        options: {
            // The layout shell is a hard-coded light theme, so following the OS
            // dark-mode preference would darken the controls and nothing else.
            darkModeSelector: false
        }
    }
});

app.use(store);
app.use(router);

app.directive('tooltip', Tooltip);
app.directive('ripple', Ripple);

app.mount('#app');
