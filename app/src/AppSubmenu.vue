<template>
  <ul v-if="items">
    <template v-for="(item,i) of items">
      <li v-if="visible(item) && !item.separator" :key="i"
          :class="[{'active-menuitem': activeIndex === i && !item.to && !item.disabled}]" role="none">
        <div v-if="item.items && root===true" class='arrow'></div>
        <router-link v-if="item.to" :to="item.to"
                     :class="[item.class, 'p-ripple',{'active-route': activeIndex === i, 'p-disabled': item.disabled}]"
                     :style="item.style"
                     @click="onMenuItemClick($event,item,i)" :target="item.target" exact role="menuitem" v-ripple>
          <i :class="item.icon"></i>
          <span>{{ item.label }}</span>
          <i v-if="item.items" class="pi pi-fw pi-angle-down menuitem-toggle-icon"></i>
          <span v-if="item.badge" class="menuitem-badge">{{ item.badge }}</span>
        </router-link>
        <a v-if="!item.to" :href="item.url||'#'" :style="item.style"
           :class="[item.class, 'p-ripple', {'p-disabled': item.disabled}]"
           @click="onMenuItemClick($event,item,i)" :target="item.target" role="menuitem" v-ripple>
          <i :class="item.icon"></i>
          <span>{{ item.label }}</span>
          <i v-if="item.items" class="pi pi-fw pi-angle-down menuitem-toggle-icon"></i>
          <span v-if="item.badge" class="menuitem-badge">{{ item.badge }}</span>
        </a>
        <transition name="layout-submenu-wrapper">
          <AppSubmenu v-show="activeIndex === i" :items="visible(item) && item.items"
                      @menuitem-click="$emit('menuitem-click', $event)"></AppSubmenu>
        </transition>
      </li>
      <li class="p-menu-separator" :style="item.style" v-if="visible(item) && item.separator" :key="'separator' + i"
          role="separator"></li>
    </template>
  </ul>
</template>

<script setup lang="ts">
import {PropType, ref} from 'vue';
import {DashboardMenu} from '@/models/dashboard';
defineProps({
  items:  Array as PropType<DashboardMenu[]>,
  root: {
    type: Boolean,
    default: false
  }
})

const emit = defineEmits(['menuitem-click']);
const activeIndex = ref(null as number | null)

function onMenuItemClick(event: Event, item: DashboardMenu, index: number) {
  if (item.disabled) {
    event.preventDefault();
    return;
  }

  if (!item.to && !item.url) {
    event.preventDefault();
  }

  //execute command
  if (item.command) {
    item.command({originalEvent: event, item: item});
  }

  activeIndex.value = index === activeIndex.value ? null : index;

  emit('menuitem-click', {
    originalEvent: event,
    item: item
  });
}

function visible(item: any) {
  return (typeof item.visible === 'function' ? item.visible() : item.visible !== false);
}
</script>

<style scoped>

</style>
