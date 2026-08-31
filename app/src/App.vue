<script setup lang="ts">
import {computed, onMounted, onUnmounted, ref, watch} from 'vue';
import {Cog, Home, Network, Terminal} from '@lucide/vue';
import AppTopbar from './AppTopbar.vue';
import AppMenu from './AppMenu.vue';
import AppFooter from './AppFooter.vue';
import Toaster from '@/components/ui/Toaster.vue';
import {useAppStore} from '@/store/appStore';
import {useInstanceStore} from '@/store/instanceStore';
import {ApiError} from '@/api/http';
import type {MenuEntry} from '@/models/dashboard';

// The 'Entities'/'Map' entries that used to live here (commented out, pointing
// at '/workspace') were removed along with that route in an earlier task in
// this plan. They were unbuilt, not unwanted: a map view is still planned --
// see .superpowers/sdd/plan-view-requirements.md -- it just has no route to
// link to yet.
const menu: MenuEntry[] = [
  {label: 'Dashboard', icon: Home, to: '/'},
  {label: 'Settings', icon: Cog, to: '/settings'},
  {label: 'RCON', icon: Terminal, to: '/rcon'},
  {label: 'LUA Script', icon: Terminal, to: '/script'},
  {label: 'Tasks', icon: Network, to: '/tasks'}
]

// jsdom reports exactly 1024, and so does a real 1024px viewport, which is the
// width Tailwind's `lg:` breakpoint starts at. Use the same comparison the
// breakpoint uses so the class toggles and the media query agree.
const isDesktop = () => window.innerWidth >= 1024

const sidebarOpen = ref(isDesktop())

function toggleSidebar() {
  sidebarOpen.value = !sidebarOpen.value
}

function onNavigate() {
  if (!isDesktop()) {
    sidebarOpen.value = false
  }
}

// An open overlay sidebar on a phone must not scroll the page behind it. This
// replaces the old layoutMode/mobileMenuActive-driven onBeforeUpdate hook:
// there is now a single sidebarOpen boolean instead of separate static/overlay
// states, so there is one place to decide whether the sidebar is acting as a
// phone-width overlay.
watch(sidebarOpen, open => {
  document.body.classList.toggle('overflow-hidden', open && !isDesktop())
})

const appStore = useAppStore()
const instanceStore = useInstanceStore()

onMounted(async () => {
  await instanceStore.checkInstanceState()
  // `maximizeWindow` is gone: sizing the OS window is not something a page in
  // a browser tab can do, and there is no HTTP route that could stand in.
  const settings = await appStore.loadSettings()
  if (settings) {
    // No REST API to start on mount any more. The server this page just
    // fetched its settings from *is* the REST API, so it is running by
    // definition; `settings.gui.enable_restapi` no longer gates anything the
    // browser can act on.
    //
    // `starting` is checked as well as `started`, which the previous version
    // did not: `checkInstanceState` returns only `started`, so reloading this
    // tab during the 8-10 minutes an extraction takes fired a second start
    // that the server could only answer 409.
    //
    // Autostart stays in the browser rather than moving to the server. It is
    // the weaker home for it -- N open tabs make N attempts where `serve`
    // would have one well-defined "once" -- but nothing under `crates/` reads
    // `enable_autostart` today (`app_settings.rs` only persists it), so
    // deleting the browser side would leave a user-visible setting with no
    // effect anywhere. The race it loses is a presentation one: the start
    // route claims its slot in a single `compare_exchange`, so exactly one
    // start happens regardless and the losers get a 409, which is handled
    // below rather than shown as a failure this tab caused.
    if (settings.gui.enable_autostart && !instanceStore.started && !instanceStore.starting) {
      try {
        await instanceStore.startInstances()
      } catch (err) {
        if (err instanceof ApiError && err.status === 409) {
          // "instance already started" or "instance is already starting" --
          // either way someone got there first and the server's view is the
          // truth. Adopting it also clears the `lastError` `startInstances`
          // just set, so this tab shows "Starting ..." rather than "Failed".
          await instanceStore.checkInstanceState()
        }
        // Anything else keeps the store's own `lastError`, which is the only
        // report a failure at mount time gets: there is no click to toast.
      }
    }
  }
  // A no-op unless a start is in flight, so an idle tab costs the one request
  // above and nothing after it. This covers the start that was already running
  // before this tab loaded; a start *this* tab makes is polled by
  // `startInstances` itself.
  instanceStore.pollWhileStarting()
})

onUnmounted(() => {
  // The poll's timer is not owned by this component's reactive scope and would
  // otherwise outlive it, holding the store and one request every two seconds.
  instanceStore.stopPolling()
  document.body.classList.remove('overflow-hidden')
})

const sidebarClasses = computed(() => sidebarOpen.value ? 'translate-x-0' : '-translate-x-full')
const shiftedClasses = computed(() => sidebarOpen.value ? 'lg:ml-sidebar' : '')
</script>

<template>
  <div class="flex min-h-screen flex-col bg-surface text-ink">
    <AppTopbar :sidebar-open="sidebarOpen" @menu-toggle="toggleSidebar"/>

    <aside
      class="fixed inset-y-0 left-0 z-40 w-sidebar overflow-y-auto bg-sidebar shadow-[0_0_6px_0_rgba(0,0,0,0.16)] transition-transform duration-200"
      :class="sidebarClasses"
      data-testid="sidebar">
      <div class="mt-6 text-center">
        <router-link to="/">
          <img alt="Logo" src="./assets/logo.png" width="250"/>
        </router-link>
      </div>
      <AppMenu :items="menu" @navigate="onNavigate"/>
    </aside>

    <div
      v-if="sidebarOpen"
      class="fixed inset-0 top-topbar z-30 bg-black/70 lg:hidden"
      data-testid="sidebar-mask"
      @click="sidebarOpen = false"></div>

    <main class="flex-1 px-8 pb-8 pt-[70px] transition-[margin] duration-200" :class="shiftedClasses">
      <router-view/>
    </main>

    <AppFooter :class="shiftedClasses"/>
    <Toaster/>
  </div>
</template>
