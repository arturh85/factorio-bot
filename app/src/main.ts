import {createApp} from 'vue';
import {createPinia} from 'pinia';
import router from './router';
import {useToast} from './composables/useToast';

import './assets/tailwind.css';

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

app.use(createPinia());
app.use(router);

app.mount('#app');
