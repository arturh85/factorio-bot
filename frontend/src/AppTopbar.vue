<template>
  <div class="layout-topbar">
    <button class="p-link layout-menu-button" @click="onMenuToggle">
      <span class="pi pi-bars"></span>
    </button>

    <div class="layout-topbar-actions" v-if="settings">
      <span>Factorio {{ settings.factorio_version }}</span> with <strong>{{ settings.client_count }} Clients</strong>
      &nbsp;
      <ProcessControl/>
    </div>


    <!--			<span class="layout-topbar-search">-->
    <!--				<InputText type="text" placeholder="Search" />-->
    <!--				<span class="layout-topbar-search-icon pi pi-search"></span>-->
    <!--			</span>-->
    <!--			<button class="p-link">-->
    <!--				<span class="layout-topbar-item-text">Events</span>-->
    <!--				<span class="layout-topbar-icon pi pi-calendar"></span>-->
    <!--				<span class="layout-topbar-badge">5</span>-->
    <!--			</button>-->
    <!--			<button class="p-link">-->
    <!--				<span class="layout-topbar-item-text">Settings</span>-->
    <!--				<span class="layout-topbar-icon pi pi-cog"></span>-->
    <!--			</button>-->
    <!--			<button class="p-link">-->
    <!--				<span class="layout-topbar-item-text">User</span>-->
    <!--				<span class="layout-topbar-icon pi pi-user"></span>-->
    <!--			</button>-->
    <!--		</div>-->
  </div>
</template>

<script>
import ProcessControl from '@/components/ProcessControl.vue'
import {computed, defineComponent} from 'vue';
import {useAppStore} from '@/store/appStore';

export default defineComponent({
  components: {
    'ProcessControl': ProcessControl
  },
  setup(props, {emit}) {
    const appStore = useAppStore();
    return {
      settings: computed(() => appStore.getSettings),
      onMenuToggle: function (event) {
        emit('menu-toggle', event);
      }
    }
  }
});
</script>