<script setup>
import {computed, watch, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import {open} from '@tauri-apps/api/dialog'
import {readDir} from '@tauri-apps/api/fs'
import InputText from 'primevue/inputtext';
import Slider from 'primevue/slider';
import Button from 'primevue/button';

const appStore = useAppStore();
const factorioArchivePath = computed({
  get() {
    return appStore.getFactorioArchivePath
  },
  set(val) {
    appStore.updateFactorioArchivePath(val)
  }
})
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
async function selectFactorioArchivePath() {
  const newPath = await open({
    defaultPath: isFactorioArchivePathValid.value ? appStore.settings.factorio_archive_path : null,
    directory: false,
    multiple: false
  })
  if (newPath) {
    await appStore.updateFactorioArchivePath(newPath)
  }
}

async function testIsWorkspacePathValid(path) {
  if (await appStore.fileExists(path)) {
    try {
      await readDir(path);
      return true
    } catch (err) {
      return false
    }
  } else {
    return false
  }
}

async function testIsFactorioArchivePathValid(path){
  return await appStore.fileExists(path)
}

const isWorkspacePathValid = ref(true)
const isFactorioArchivePathValid = ref(true)

if (appStore.settings) {
  watch(() => appStore.getWorkspacePath, async () => {
    isWorkspacePathValid.value = await testIsWorkspacePathValid(appStore.settings.workspace_path)
  })
  watch(() => appStore.getFactorioArchivePath, async () => {
    isFactorioArchivePathValid.value = await testIsFactorioArchivePathValid(appStore.settings.factorio_archive_path)
  })
  testIsWorkspacePathValid(appStore.settings.workspace_path).then(valid => isWorkspacePathValid.value = valid)
  testIsFactorioArchivePathValid(appStore.settings.factorio_archive_path).then(valid => isFactorioArchivePathValid.value = valid)
}

const openInBrowser = (url, event) => {
  appStore.openInBrowser(url);
  if (event) {
    event.preventDefault()
  }
  return false
}
const settings = computed(() => appStore.getSettings)
</script>

<template>
  <div class="p-grid" v-if="settings">
    <div class="p-col-12">
      <div class="card">
        <h5>Settings</h5>
        <p>Use this page to start from scratch and place your custom content.</p>
      </div>


      <div class="card p-fluid">
        <h5>Factorio Archive - Download from <a href="https://factorio.com/download" @click="openInBrowser('https://factorio.com/download', $event)">https://factorio.com/download</a></h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <InputText v-model="factorioArchivePath" :class="isFactorioArchivePathValid ? '' : 'p-invalid'"/>
              <Button label="Select" @click="selectFactorioArchivePath()"/>
            </div>
          </div>
        </div>
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
