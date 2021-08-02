<script>
import {computed, defineComponent, watch, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import {open} from '@tauri-apps/api/dialog'
import {readDir} from '@tauri-apps/api/fs'
import InputText from 'primevue/inputtext';
import Slider from 'primevue/slider';
import Button from 'primevue/button';

export default defineComponent({
  components: {
    InputText,
    Slider,
    Button
  },
  setup(props, {emit}) {
    const appStore = useAppStore();
    const workspacePath = computed({
      get() {
        return appStore.getWorkspacePath
      },
      set(val) {
        appStore.updateWorkspacePath(val)
      }
    })

    const clientCount = computed({
      get() {
        return appStore.getClientCount
      },
      set(val) {
        appStore.updateClientCount(val)
      }
    })
    const mapExchangeString = computed({
      get() {
        return appStore.getMapExchangeString
      },
      set(val) {
        appStore.updateMapExchangeString(val)
      }
    })
    const seed = computed({
      get() {
        return appStore.getSeed
      },
      set(val) {
        appStore.updateSeed(val)
      }
    })

    async function selectWorkspacePath() {
      const newPath = await open({
        defaultPath: isWorkspacePathValid.value ? appStore.settings.workspace_path : null,
        directory: true,
        multiple: false
      })
      if (newPath) {
        await appStore.updateWorkspacePath(newPath)
      }
    }

    async function testIsWorkspacePathValid(path) {
      try {
        await readDir(path);
        return true
      } catch (err) {
        return false
      }
    }

    const isWorkspacePathValid = ref(true)

    if (appStore.settings) {
      watch(() => appStore.getWorkspacePath, async () => {
        isWorkspacePathValid.value = await testIsWorkspacePathValid(appStore.settings.workspace_path)
      })
      testIsWorkspacePathValid(appStore.settings.workspace_path).then(valid => isWorkspacePathValid.value = valid)
    }

    return {
      mapExchangeString,
      seed,
      selectWorkspacePath,
      isWorkspacePathValid,
      workspacePath,
      clientCount,
      settings: computed(() => appStore.getSettings),
      availableFactorioVersions: computed(() => factorioVersionsStore.getFactorioVersions.map(version => ({
        name: version, code: version
      }))),
      onMenuToggle: function (event) {
        emit('menu-toggle', event);
      }
    }
  }
});
</script>

<template>
  <div class="p-grid" v-if="settings">
    <div class="p-col-12">
      <div class="card">
        <h5>Settings</h5>
        <p>Use this page to start from scratch and place your custom content.</p>
      </div>

      <div class="card p-fluid">
        <h5>Map Exchange String</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <InputText v-model="mapExchangeString"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Seed</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <InputText v-model="seed"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Number of Factorio Client Instances</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="client_count">Client Instances: {{ clientCount }}</label>
            <Slider id="client_count" v-model="clientCount" :min="0" :max="16"/>
          </div>
        </div>
      </div>

      <div class="card p-fluid">
        <h5>Workspace Folder</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <InputText v-model="workspacePath" :class="isWorkspacePathValid ? '' : 'p-invalid'"/>
              <Button label="Select" @click="selectWorkspacePath()"/>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>

</style>
