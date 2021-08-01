<template>
  <div class="p-grid" v-if="settings">
    <div class="p-col-12">
      <div class="card">
        <h5>Settings</h5>
        <p>Use this page to start from scratch and place your custom content.</p>
      </div>

      <div class="card p-fluid">
        <h5>Workspace Folder</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <InputText v-model="workspacePath" :class="isWorkspacePathValid ? '' : 'p-invalid'" />
              <Button label="Select" @click="selectWorkspacePath()"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Factorio Version</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="factorio_version">Version</label>
            <Dropdown id="factorio_version" v-model="factorioVersion" :options="availableFactorioVersions"
                      optionLabel="name" placeholder="Select One"></Dropdown>
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
    </div>
  </div>
</template>

<script lang="ts">
import {computed, defineComponent, watch, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import {useFactorioVersionsStore} from '@/store/factorioVersionsStore';
import {open} from '@tauri-apps/api/dialog'
import {readDir} from '@tauri-apps/api/fs'

export default defineComponent({
  setup(props, {emit}) {
    const appStore = useAppStore();
    const factorioVersionsStore = useFactorioVersionsStore();
    factorioVersionsStore.loadFactorioVersions()

    const workspacePath = computed({
      get() {
        return appStore.getWorkspacePath
      },
      set(val) {
        appStore.updateWorkspacePath(val)
      }
    })

    const factorioVersion = computed({
      get() {
        const v = appStore.getFactorioVersion
        return {code: v, name: v}
      },
      set(val) {
        appStore.updateFactorioVersion(val.code)
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

    async function selectWorkspacePath() {
      const new_path = await open({
        defaultPath: isWorkspacePathValid.value ? appStore.settings.workspace_path : null,
        directory: true,
        multiple: false
      })
      if (new_path) {
        await appStore.updateWorkspacePath(new_path)
      }
    }

    async function testIsWorkspacePathValid(path) {
      console.log('isWorkspacePathValid', path)
      try {
        const result = await readDir(path);
        console.log('RESULT', result)
        return true
      } catch(err) {
        console.log('err', err)
        return false
      }
    }

    const isWorkspacePathValid = ref(true)

    if (appStore.settings) {
      watch(() => appStore.getWorkspacePath, async () => {
        console.log('watch called')
        isWorkspacePathValid.value = await testIsWorkspacePathValid(appStore.settings.workspace_path)
      })
      testIsWorkspacePathValid(appStore.settings.workspace_path).then(valid => isWorkspacePathValid.value = valid)
    }

    return {
      selectWorkspacePath,
      isWorkspacePathValid,
      factorioVersion,
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

<style scoped>

</style>
