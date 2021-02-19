<template>
  <div :class="containerClass" @click="onWrapperClick">
    <AppTopBar @menu-toggle="onMenuToggle"/>
    <transition name="layout-sidebar">
      <div :class="sidebarClass"
           @click="onSidebarClick"
           v-show="isSidebarVisible()">
        <div class="layout-logo">
          <router-link to="/">
            <img alt="Logo" :src="logo" :width="100"/>
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
import AppConfig from './AppConfig.vue'
import AppFooter from './AppFooter.vue'

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
        {label: 'Instances', icon: 'pi pi-fw pi-circle-off', to: '/instances'},
        {
          label: 'UI Kit', icon: 'pi pi-fw pi-sitemap',
          items: [
            {label: 'Form Layout', icon: 'pi pi-fw pi-id-card', to: '/formlayout'},
            {label: 'Input', icon: 'pi pi-fw pi-check-square', to: '/input'},
            {label: 'Float Label', icon: 'pi pi-fw pi-bookmark', to: '/floatlabel'},
            {label: 'Button', icon: 'pi pi-fw pi-mobile', to: '/button'},
            {label: 'Table', icon: 'pi pi-fw pi-table', to: '/table'},
            {label: 'List', icon: 'pi pi-fw pi-list', to: '/list'},
            {label: 'Tree', icon: 'pi pi-fw pi-share-alt', to: '/tree'},
            {label: 'Panel', icon: 'pi pi-fw pi-tablet', to: '/panel'},
            {label: 'Overlay', icon: 'pi pi-fw pi-clone', to: '/overlay'},
            {label: 'Menu', icon: 'pi pi-fw pi-bars', to: '/menu'},
            {label: 'Message', icon: 'pi pi-fw pi-comment', to: '/messages'},
            {label: 'Chart', icon: 'pi pi-fw pi-chart-bar', to: '/chart'},
            {label: 'Misc', icon: 'pi pi-fw pi-circle-off', to: '/misc'},
            {label: 'Display', icon: 'pi pi-fw pi-desktop', to: '/display'},
            {label: 'Elevation', icon: 'pi pi-fw pi-external-link', to: '/elevation'},
            {label: 'Flexbox', icon: 'pi pi-fw pi-directions', to: '/flexbox'},
            {label: 'Icons', icon: 'pi pi-fw pi-search', to: '/icons'},
            {label: 'Grid System', icon: 'pi pi-fw pi-th-large', to: '/grid'},
            {label: 'Spacing', icon: 'pi pi-fw pi-arrow-right', to: '/spacing'},
            {label: 'Typography', icon: 'pi pi-fw pi-align-center', to: '/typography'},
            {label: 'Text', icon: 'pi pi-fw pi-pencil', to: '/text'},
          ],
        },
        {
          label: 'View Source', icon: 'pi pi-fw pi-search', command: () => {
            window.location = 'https://gitlab.app42.blue/dokku/factorio-bot-rs'
          },
        },
      ],
    }
  },
  watch: {
    $route() {
      this.menuActive = false
      this.$toast.removeAllGroups()
    },
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
    },
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
        'p-ripple-disabled': this.$primevue.ripple === false,
      }]
    },
    sidebarClass() {
      return ['layout-sidebar', {
        'layout-sidebar-dark': this.layoutColorMode === 'dark',
        'layout-sidebar-light': this.layoutColorMode === 'light',
      }]
    },
    logo() {
      // return (this.layoutColorMode === 'dark') ? "assets/layout/images/logo-white.svg" : "assets/layout/images/logo.svg";
      return 'logo.png'
    },
  },
  beforeUpdate() {
    if (this.mobileMenuActive)
      this.addClass(document.body, 'body-overflow-hidden')
    else
      this.removeClass(document.body, 'body-overflow-hidden')
  },
  components: {
    'AppTopBar': AppTopBar,
    'AppMenu': AppMenu,
    'AppConfig': AppConfig,
    'AppFooter': AppFooter,
  },
}
</script>

<style lang="scss">
@import './App.scss';
</style>
