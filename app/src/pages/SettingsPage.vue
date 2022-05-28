<script setup lang="ts">
import {computed, watch, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import {open} from '@tauri-apps/api/dialog'
import {readDir} from '@tauri-apps/api/fs'
import InputText from 'primevue/inputtext';
import Slider from 'primevue/slider';
import Button from 'primevue/button';
import Checkbox from 'primevue/checkbox';
import {useRestApiStore} from '@/store/restapiStore';

const appStore = useAppStore();
const restApiStore = useRestApiStore();
const factorioArchivePath = computed({
  get(): string {
    return appStore.getFactorioArchivePath as string
  },
  set(val: string) {
    appStore.updateFactorioArchivePath(val)
  }
})
const workspacePath = computed({
  get(): string {
    return appStore.getWorkspacePath as string
  },
  set(val: string) {
    appStore.updateWorkspacePath(val)
  }
})
const clientCount = computed({
  get(): number {
    return appStore.getClientCount as number
  },
  set(val: number) {
    appStore.updateClientCount(val)
  }
})
const recreateLevel = computed({
  get(): boolean {
    return appStore.getRecreateLevel as boolean
  },
  set(val: boolean) {
    appStore.updateRecreateLevel(val)
  }
})
const enableRestapi = computed({
  get(): boolean {
    return appStore.getEnableRestapi as boolean
  },
  set(val: boolean) {
    appStore.updateEnableRestApi(val)
  }
})
const enableAutostart = computed({
  get(): boolean {
    return appStore.getEnableAutostart as boolean
  },
  set(val: boolean) {
    appStore.updateEnableAutostart(val)
  }
})
const restapiPort = computed({
  get(): string {
    return appStore.getRestapiPort as any
  },
  set(val: string) {
    appStore.updateRestapiPort(parseInt(val, 10))
  }
})
const mapExchangeString = computed({
  get(): string {
    return appStore.getMapExchangeString as string
  },
  set(val: string) {
    appStore.updateMapExchangeString(val)
  }
})
const seed = computed({
  get(): string {
    return appStore.getSeed as string
  },
  set(val: string) {
    appStore.updateSeed(val)
  }
})

async function selectWorkspacePath() {
  if (!appStore.settings) {
    throw new Error('missing settings')
  }
  const newPath = await open({
    defaultPath: isWorkspacePathValid.value ? appStore.settings.factorio.workspace_path : undefined,
    directory: true,
    multiple: false
  })
  if (newPath) {
    await appStore.updateWorkspacePath(newPath as string)
  }
}

async function selectFactorioArchivePath() {
  if (!appStore.settings) {
    throw new Error('missing settings')
  }
  const newPath = await open({
    defaultPath: isFactorioArchivePathValid.value ? appStore.settings.factorio.factorio_archive_path : undefined,
    directory: false,
    multiple: false
  })
  if (newPath) {
    await appStore.updateFactorioArchivePath(newPath as string)
  }
}

async function testIsWorkspacePathValid(path: string) {
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

async function testIsFactorioArchivePathValid(path: string): Promise<boolean> {
  return await appStore.fileExists(path) as boolean
}

const isWorkspacePathValid = ref(true)
const isFactorioArchivePathValid = ref(true)
const isPortAvaiable = computed(() => restApiStore.isPortAvailable)

if (appStore.settings) {
  watch(() => appStore.getWorkspacePath, async () => {
    if (appStore.settings) {
      isWorkspacePathValid.value = await testIsWorkspacePathValid(appStore.settings.factorio.workspace_path)
    }
  })
  watch(() => appStore.getFactorioArchivePath, async () => {
    if (appStore.settings) {
      isFactorioArchivePathValid.value = await testIsFactorioArchivePathValid(appStore.settings.factorio.factorio_archive_path)
    }
  })
  testIsWorkspacePathValid(appStore.settings.factorio.workspace_path).then(valid => isWorkspacePathValid.value = valid)
  testIsFactorioArchivePathValid(appStore.settings.factorio.factorio_archive_path).then(valid => isFactorioArchivePathValid.value = valid)
}

const openInBrowser = (url: string, event: Event) => {
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
        <h5>Factorio Archive - Download from <a href="https://factorio.com/download"
                                                @click="openInBrowser('https://factorio.com/download', $event)">https://factorio.com/download</a>
        </h5>
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
        <h5>Recreate Level on Start</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <Checkbox v-model="recreateLevel" :binary="true" label="Recreate Level"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Autostart Factorio</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <Checkbox v-model="enableAutostart" :binary="true" label="Enable Autostart"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Enable REST API
          <a v-if="enableRestapi" :href="'http://localhost:' + restapiPort + '/swagger-ui/'"
             @click="openInBrowser('http://localhost:' + restapiPort, $event)">{{ 'http://localhost:' + restapiPort }}</a>
          &nbsp;
          <a v-if="enableRestapi" :href="'http://localhost:' + restapiPort + '/swagger-ui/'"
             @click="openInBrowser('http://localhost:' + restapiPort + '/swagger-ui/', $event)">{{ 'swagger-ui' }}</a>
        </h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              <Checkbox v-model="enableRestapi" :binary="true" label="Enable"/>
              Port
              <InputText v-model="restapiPort"  :class="isPortAvaiable ? '' : 'p-invalid'" type="number" min="1" max="65535" />
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
