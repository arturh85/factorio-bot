<template>
  <div ref="root" id="layout-config" :class="containerClass">
    <a href="#" class="layout-config-button" id="layout-config-button" @click="toggleConfigurator">
      <i class="pi pi-cog"></i>
    </a>
    <a href="#" class="layout-config-close" @click="hideConfigurator">
      <i class="pi pi-times"></i>
    </a>

    <div class="layout-config-content">

      <h5 style="margin-top: 0">Input Style</h5>
      <div class="p-formgroup-inline">
        <div class="p-field-radiobutton">
          <RadioButton id="input_outlined" name="inputstyle" value="outlined" :modelValue="inputStyle"
                       @update:modelValue="changeInputStyle"/>
          <label for="input_outlined">Outlined</label>
        </div>
        <div class="p-field-radiobutton">
          <RadioButton id="input_filled" name="inputstyle" value="filled" :modelValue="inputStyle"
                       @update:modelValue="changeInputStyle"/>
          <label for="input_filled">Filled</label>
        </div>
      </div>

      <h5>Ripple Effect</h5>
      <InputSwitch :modelValue="rippleActive" @update:modelValue="changeRipple"/>

      <h5>Menu Type</h5>
      <div class="p-formgroup-inline">
        <div class="p-field-radiobutton">
          <RadioButton id="static" name="layoutMode" value="static" v-model="d_layoutMode"
                       @change="changeLayout($event, 'static')"/>
          <label for="static">Static</label>
        </div>
        <div class="p-field-radiobutton">
          <RadioButton id="overlay" name="layoutMode" value="overlay" v-model="d_layoutMode"
                       @change="changeLayout($event, 'overlay')"/>
          <label for="overlay">Overlay</label>
        </div>
      </div>

      <h5>Menu Color</h5>
      <div class="p-formgroup-inline">
        <div class="p-field-radiobutton">
          <RadioButton id="dark" name="layoutColorMode" value="dark" v-model="d_layoutColorMode"
                       @change="changeLayoutColor($event, 'dark')"/>
          <label for="dark">Dark</label>
        </div>
        <div class="p-field-radiobutton">
          <RadioButton id="light" name="layoutColorMode" value="light" v-model="d_layoutColorMode"
                       @change="changeLayoutColor($event, 'light')"/>
          <label for="light">Light</label>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import RadioButton from 'primevue/radiobutton';
import InputSwitch from 'primevue/inputswitch';

import {ref, computed} from 'vue';

const props = defineProps({
  layoutMode: {
    type: String,
    default: null
  },
  layoutColorMode: {
    type: String,
    default: null
  }
})
const root = ref(null as Element | null)
const active = ref(false)
const d_layoutMode = ref(props.layoutMode)
const d_layoutColorMode = ref(props.layoutColorMode)
// watch: {
// 	$route() {
// 		if (this.active) {
// 			this.active = false;
// 			this.unbindOutsideClickListener();
// 		}
// 	},
// function layoutMode(newValue: string) {
//   d_layoutMode.value = newValue;
// }
//
// function layoutColorMode(newValue: string) {
//   d_layoutColorMode.value = newValue;
// }

const outsideClickListener = ref(null as any)

function toggleConfigurator(event: CustomEvent<void>) {
  active.value = !active.value;
  event.preventDefault();

  if (active.value)
    bindOutsideClickListener();
  else
    unbindOutsideClickListener();
}

function hideConfigurator(event: CustomEvent<void>) {
  active.value = false;
  unbindOutsideClickListener();
  event.preventDefault();
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
function changeInputStyle() {
  // this.$appState.inputStyle = value;
}

// eslint-disable-next-line @typescript-eslint/no-unused-vars
function changeRipple() {
  // this.$primevue.ripple = value;
}

const emit = defineEmits(['layout-change', 'layout-color-change']);
function changeLayout(event: CustomEvent<void>, layoutMode: string) {
  emit('layout-change', layoutMode);
  event.preventDefault();
}

function changeLayoutColor(event: CustomEvent<void>, layoutColor: string) {
  emit('layout-color-change', layoutColor);
  event.preventDefault();
}

function bindOutsideClickListener() {
  if (!outsideClickListener.value) {
    outsideClickListener.value = (event: CustomEvent<void>) => {
      if (active.value && isOutsideClicked(event)) {
        active.value = false;
      }
    };
    document.addEventListener('click', outsideClickListener.value);
  }
}

function unbindOutsideClickListener() {
  if (outsideClickListener.value) {
    document.removeEventListener('click', outsideClickListener.value);
    outsideClickListener.value = null;
  }
}

function isOutsideClicked(event: CustomEvent<void>) {
  if (!root.value) {
    return false
  }

  return !(root.value.isSameNode(event.target as any) || root.value.contains(event.target as any))
}

const containerClass = computed(() => {
  return ['layout-config', {'layout-config-active': active.value}];
})
const rippleActive = computed(() => {
  // return this.$primevue.ripple;
  return false
})
const inputStyle = computed(() => {
  // return this.$appState.inputStyle;
  return false
})
</script>
