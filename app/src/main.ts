import { createApp } from 'vue';
import { reactive } from 'vue';
import router from './router';
import {createPinia} from 'pinia';
import ToastService from 'primevue/toastservice';
import Tooltip from 'primevue/tooltip';
import Ripple from 'primevue/ripple';

import 'primevue/resources/themes/saga-blue/theme.css';
import 'primevue/resources/primevue.min.css';
import 'primeflex/primeflex.css';
import 'primeicons/primeicons.css';
import './assets/layout/layout.scss';
import './assets/layout/flags/flags.css';

import './plugins/configure-ynetwork';
import App from './App.vue';

// Vue Router 5 deprecates the next() callback; returning undefined continues
// the navigation, and next() is removed entirely in Router 6.
router.beforeEach(() => {
    window.scrollTo(0, 0);
});

const app = createApp(App);
const store = createPinia()

app.config.globalProperties.$appState = reactive({ inputStyle: 'outlined' });
app.config.globalProperties.$primevue = reactive({ ripple: true, config: {zIndex: {}} });

app.use(ToastService);
app.use(store);
app.use(router);

app.directive('tooltip', Tooltip);
app.directive('ripple', Ripple);

app.mount('#app');
