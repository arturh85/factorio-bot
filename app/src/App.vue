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
import {useRestApiStore} from '@/store/restapiStore';
import {useInstanceStore} from '@/store/instanceStore';
import {computed, onBeforeUpdate, onMounted, ref} from 'vue';
import {onBeforeRouteLeave} from 'vue-router';
import {useToast} from 'primevue/usetoast';
import {DashboardMenu} from "@/models/dashboard";

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
      {label: 'Tasks', icon: 'pi pi-fw pi-sitemap', to: '/tasks'},
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
    'layout-mobile-sidebar-active': mobileMenuActive.value,
    'p-input-filled': true,
    'p-ripple-disabled': false
  }]
})
const sidebarClass = computed(() => {
  return ['layout-sidebar', {
    'layout-sidebar-dark': layoutColorMode.value === 'dark',
    'layout-sidebar-light': layoutColorMode.value === 'light'
  }]
})

onMounted(async () => {
  const instanceStore = useInstanceStore()
  const started = await instanceStore.checkInstanceState()
  const appStore = useAppStore()
  await appStore.maximizeWindow()
  const settings = await appStore.loadSettings()
  if (settings) {
    if (settings.gui.enable_restapi) {
      const restApiStore = useRestApiStore()
      await restApiStore.init()
      if (!restApiStore.started) {
        await restApiStore.startRestApi()
      }
    }
    if (!started && settings.gui.enable_autostart) {
      await instanceStore.startInstances();
    }
  }
})

onBeforeUpdate(() => {
  if (mobileMenuActive.value)
    addClass(document.body, 'body-overflow-hidden')
  else
    removeClass(document.body, 'body-overflow-hidden')
});
</script>

<style lang="scss">
.p-toast.p-toast-topright {
  z-index: 1000;
  top: 70px;
}
</style>
