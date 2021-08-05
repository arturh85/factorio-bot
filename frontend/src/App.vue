<template>
  <div :class="containerClass" @click="onWrapperClick">
    <AppTopBar @menu-toggle="onMenuToggle"/>
    <transition name="layout-sidebar">
      <div :class="sidebarClass"
           @click="onSidebarClick"
           v-show="isSidebarVisible()">
        <div class="layout-logo">
          <router-link to="/">
            <img alt="Logo" src="./assets/logo.png" :width="100"/>
            <h2>Factorio Bot Platform</h2>
          </router-link>
        </div>

        <AppMenu :model="menu"
                 @menuitem-click="onMenuItemClick"/>
      </div>
    </transition>

    <div class="layout-main">
      <router-view/>
    </div>

        <AppConfig :layoutMode="layoutMode"
                   :layoutColorMode="layoutColorMode"
                   @layout-change="onLayoutChange"
                   @layout-color-change="onLayoutColorChange"/>

    <AppFooter/>
  </div>
</template>

<script>
import AppTopBar from './AppTopbar.vue'
import AppMenu from './AppMenu.vue'
import AppFooter from './AppFooter.vue'
import {useAppStore} from '@/store/appStore';
import AppConfig from '@/AppConfig.vue';

export default {
  data() {
    return {
      layoutMode: 'static',
      layoutColorMode: 'dark',
      staticMenuInactive: false,
      overlayMenuActive: false,
      mobileMenuActive: false,
      menu: [
        {label: 'Dashboard', icon: 'pi pi-fw pi-home', to: '/'},
        {label: 'Settings', icon: 'pi pi-fw pi-cog', to: '/settings'},
        {label: 'RCON', icon: 'pi pi-fw pi-cog', to: '/rcon'},
        {label: 'LUA Script', icon: 'pi pi-fw pi-cog', to: '/script'},
        {label: 'Mods', icon: 'pi pi-fw pi-th-large', to: '/factorioMods'},
        {label: 'Tasks', icon: 'pi pi-fw pi-sitemap', to: '/workspace'},
        {label: 'Entities', icon: 'pi pi-fw pi-sitemap', to: '/workspace'},
        {label: 'Map', icon: 'pi pi-fw pi-map-marker', to: '/workspace'},
        {label: 'Instances', icon: 'pi pi-fw pi-circle-off', to: '/instances'},
        {label: 'REST API Docs', icon: 'pi pi-fw pi-question-circle', to: '/restApiDocss'},
        {label: 'LUA API Docs', icon: 'pi pi-fw pi-question-circle', to: '/luaApiDocss'}
      ]
    }
  },
  watch: {
    $route() {
      this.menuActive = false
      this.$toast.removeAllGroups()
    }
  },
  methods: {
    onWrapperClick() {
      if (!this.menuClick) {
        this.overlayMenuActive = false
        this.mobileMenuActive = false
      }

      this.menuClick = false
    },
    onMenuToggle() {
      this.menuClick = true

      if (this.isDesktop()) {
        if (this.layoutMode === 'overlay') {
          if (this.mobileMenuActive === true) {
            this.overlayMenuActive = true
          }

          this.overlayMenuActive = !this.overlayMenuActive
          this.mobileMenuActive = false
        } else if (this.layoutMode === 'static') {
          this.staticMenuInactive = !this.staticMenuInactive
        }
      } else {
        this.mobileMenuActive = !this.mobileMenuActive
      }

      event.preventDefault()
    },
    onSidebarClick() {
      this.menuClick = true
    },
    onMenuItemClick(event) {
      if (event.item && !event.item.items) {
        this.overlayMenuActive = false
        this.mobileMenuActive = false
      }
    },
    onLayoutChange(layoutMode) {
      this.layoutMode = layoutMode
    },
    onLayoutColorChange(layoutColorMode) {
      this.layoutColorMode = layoutColorMode
    },
    addClass(element, className) {
      if (element.classList)
        element.classList.add(className)
      else
        element.className += ' ' + className
    },
    removeClass(element, className) {
      if (element.classList)
        element.classList.remove(className)
      else
        element.className = element.className.replace(new RegExp('(^|\\b)' + className.split(' ').join('|') + '(\\b|$)', 'gi'), ' ')
    },
    isDesktop() {
      return window.innerWidth > 1024
    },
    isSidebarVisible() {
      if (this.isDesktop()) {
        if (this.layoutMode === 'static')
          return !this.staticMenuInactive
        else if (this.layoutMode === 'overlay')
          return this.overlayMenuActive
        else
          return true
      } else {
        return true
      }
    }
  },
  computed: {
    containerClass() {
      return ['layout-wrapper', {
        'layout-overlay': this.layoutMode === 'overlay',
        'layout-static': this.layoutMode === 'static',
        'layout-static-sidebar-inactive': this.staticMenuInactive && this.layoutMode === 'static',
        'layout-overlay-sidebar-active': this.overlayMenuActive && this.layoutMode === 'overlay',
        'layout-mobile-sidebar-active': this.mobileMenuActive,
        'p-input-filled': this.$appState.inputStyle === 'filled',
        'p-ripple-disabled': this.$primevue.ripple === false
      }]
    },
    sidebarClass() {
      return ['layout-sidebar', {
        'layout-sidebar-dark': this.layoutColorMode === 'dark',
        'layout-sidebar-light': this.layoutColorMode === 'light'
      }]
    },
    logo() {
      // return (this.layoutColorMode === 'dark') ? "assets/layout/images/logo-white.svg" : "assets/layout/images/logo.svg";
      return 'assets/logo.png'
    }
  },
  async created() {
    const appStore = useAppStore()
    await appStore.maximizeWindow()
    await appStore.loadSettings()
  },
  beforeUpdate() {
    if (this.mobileMenuActive)
      this.addClass(document.body, 'body-overflow-hidden')
    else
      this.removeClass(document.body, 'body-overflow-hidden')
  },
  components: {
    AppTopBar,
    AppMenu,
    AppFooter,
    AppConfig
  }
}
</script>

<style lang="scss">
@import './App.scss';
</style>
