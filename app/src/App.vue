<template>
  <div :class="containerClass" @click="onWrapperClick">
    <AppTopBar @menu-toggle="onMenuToggle"/>
    <transition name="layout-sidebar">
      <div :class="sidebarClass"
           @click="onSidebarClick"
           v-show="isSidebarVisible()">
        <div class="layout-logo">
          <router-link to="/">
            <img alt="Logo" src="./assets/logo.png" :width="250"/>

          </router-link>
        </div>

        <AppMenu :model="menu"
                 @menuitem-click="onMenuItemClick"/>
      </div>
    </transition>

    <div class="layout-main">
      <router-view/>
      <Toast position="bottom-right"/>
    </div>

    <AppConfig :layoutMode="layoutMode"
               :layoutColorMode="layoutColorMode"
               @layout-change="onLayoutChange"
               @layout-color-change="onLayoutColorChange"/>

    <AppFooter/>
  </div>
</template>

<script setup lang="ts">
import AppTopBar from './AppTopbar.vue'
import AppMenu from './AppMenu.vue'
import AppFooter from './AppFooter.vue'
import {useAppStore} from '@/store/appStore';
import AppConfig from '@/AppConfig.vue';
import Toast from 'primevue/toast';
import {useInstanceStore} from '@/store/instanceStore';
import {computed, onBeforeUpdate, onMounted, onUnmounted, ref} from 'vue';
import {ApiError} from '@/api/http';
import {onBeforeRouteLeave} from 'vue-router';
import {useToast} from 'primevue/usetoast';
import {DashboardMenu} from '@/models/dashboard';

const layoutMode = ref('static')
const layoutColorMode = ref('dark')
const staticMenuInactive = ref(false)
const overlayMenuActive = ref(false)
const mobileMenuActive = ref(false)
const menuClick = ref(false)
const menu = ref([
      {label: 'Dashboard', icon: 'pi pi-fw pi-home', to: '/'},
      {label: 'Settings', icon: 'pi pi-fw pi-cog', to: '/settings'},
      {label: 'RCON', icon: 'pi pi-fw pi-cog', to: '/rcon'},
      {label: 'LUA Script', icon: 'pi pi-fw pi-cog', to: '/script'},
      // {label: 'Mods', icon: 'pi pi-fw pi-th-large', to: '/factorioMods'},
      {label: 'Tasks', icon: 'pi pi-fw pi-sitemap', to: '/tasks'}
      // {label: 'Entities', icon: 'pi pi-fw pi-sitemap', to: '/workspace'},
      // {label: 'Map', icon: 'pi pi-fw pi-map-marker', to: '/workspace'},
      // {label: 'Instances', icon: 'pi pi-fw pi-circle-off', to: '/instances'},
      // {label: 'REST API Docs', icon: 'pi pi-fw pi-question-circle', to: '/restApiDocss'},
      // {label: 'LUA API Docs', icon: 'pi pi-fw pi-question-circle', to: '/luaApiDocss'}
    ] as DashboardMenu[]
)

onBeforeRouteLeave(() => {
  // menuActive.value = false

  const toast = useToast()
  toast.removeAllGroups()
})

function onWrapperClick() {
  if (!menuClick.value) {
    overlayMenuActive.value = false
    mobileMenuActive.value = false
  }

  menuClick.value = false
}

function onMenuToggle(event: CustomEvent<void>) {
  menuClick.value = true

  if (isDesktop()) {
    if (layoutMode.value === 'overlay') {
      if (mobileMenuActive.value === true) {
        overlayMenuActive.value = true
      }

      overlayMenuActive.value = !overlayMenuActive.value
      mobileMenuActive.value = false
    } else if (layoutMode.value === 'static') {
      staticMenuInactive.value = !staticMenuInactive.value
    }
  } else {
    mobileMenuActive.value = !mobileMenuActive.value
  }

  event.preventDefault()
}

function onSidebarClick() {
  menuClick.value = true
}

function onMenuItemClick(event: any) {
  if (event.item && !event.item.items) {
    overlayMenuActive.value = false
    mobileMenuActive.value = false
  }
}

function onLayoutChange(_layoutMode: string) {
  layoutMode.value = _layoutMode
}

function onLayoutColorChange(_layoutColorMode: string) {
  layoutColorMode.value = _layoutColorMode
}

function addClass(element: Element, className: string) {
  if (element.classList)
    element.classList.add(className)
  else
    element.className += ' ' + className
}

function removeClass(element: Element, className: string) {
  if (element.classList)
    element.classList.remove(className)
  else
    element.className = element.className.replace(new RegExp('(^|\\b)' + className.split(' ').join('|') + '(\\b|$)', 'gi'), ' ')
}

function isDesktop() {
  return window.innerWidth > 1024
}

function isSidebarVisible() {
  if (isDesktop()) {
    if (layoutMode.value === 'static')
      return !staticMenuInactive.value
    else if (layoutMode.value === 'overlay')
      return overlayMenuActive.value
    else
      return true
  } else {
    return true
  }
}

const containerClass = computed(() => {
  return ['layout-wrapper', {
    'layout-overlay': layoutMode.value === 'overlay',
    'layout-static': layoutMode.value === 'static',
    'layout-static-sidebar-inactive': staticMenuInactive.value && layoutMode.value === 'static',
    'layout-overlay-sidebar-active': overlayMenuActive.value && layoutMode.value === 'overlay',
    'layout-mobile-sidebar-active': mobileMenuActive.value
  }]
})
const sidebarClass = computed(() => {
  return ['layout-sidebar', {
    'layout-sidebar-dark': layoutColorMode.value === 'dark',
    'layout-sidebar-light': layoutColorMode.value === 'light'
  }]
})

const instanceStore = useInstanceStore()

onMounted(async () => {
  await instanceStore.checkInstanceState()
  const appStore = useAppStore()
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
})

onBeforeUpdate(() => {
  if (mobileMenuActive.value)
    addClass(document.body, 'body-overflow-hidden')
  else
    removeClass(document.body, 'body-overflow-hidden')
});
</script>

<style lang="scss">
.p-toast.p-toast-bottom-right {
  z-index: 1000;
}
</style>
