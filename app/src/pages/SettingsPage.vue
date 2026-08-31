<script setup lang="ts">
import {computed, watch, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import InputText from 'primevue/inputtext';
import Slider from 'primevue/slider';
import Checkbox from 'primevue/checkbox';

const appStore = useAppStore();
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

/**
 * Both configured paths name the *server's* filesystem, so only the server can
 * say whether they exist.
 *
 * This is why the two "Select" buttons are gone rather than ported. They
 * opened a Tauri native picker, which browses the machine the *UI* runs on --
 * already the wrong disk whenever the server was remote, and not something a
 * browser can do at all: a file input hands back a `File`, never a path. A
 * text field checked against `GET /api/v1/fs/exists` is the honest
 * replacement.
 *
 * An empty path is reported invalid, not skipped: "" is not a directory on the
 * server either, and the field is only rendered once settings have loaded.
 */
async function existsOnServer(path: string | null): Promise<boolean> {
  return path ? appStore.fileExists(path) : false
}

const isWorkspacePathValid = ref(true)
const isFactorioArchivePathValid = ref(true)

// The port field no longer shows an "already in use" invalid state. The check
// behind it was Tauri's `is_port_available`, an in-process probe of the host
// the UI ran on; a browser cannot probe the server's ports, and there is no
// HTTP route that could stand in. Adding one would not restore the guarantee
// either -- the server binds the port at the next `factorio-bot serve`, so a
// port free at check time can be taken by then. A clash therefore surfaces at
// that start, and the field is persisted validation-free.

// Registered unconditionally and with `immediate`. Guarding this on
// `appStore.settings` -- which is null on the first render whenever the page
// is opened directly, since `App.vue` loads settings asynchronously -- left
// the watchers unregistered for the life of the component, so neither field
// was ever validated.
watch(() => appStore.getWorkspacePath, async (path) => {
  isWorkspacePathValid.value = await existsOnServer(path)
}, {immediate: true})
watch(() => appStore.getFactorioArchivePath, async (path) => {
  isFactorioArchivePathValid.value = await existsOnServer(path)
}, {immediate: true})

// `openInBrowser` is gone with the rest of the Tauri surface. The links below
// are plain anchors now -- a page that is already in a browser does not need a
// host command to open one.
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
                                                target="_blank"
                                                rel="noopener noreferrer">https://factorio.com/download</a>
        </h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <InputText v-model="factorioArchivePath" :class="isFactorioArchivePathValid ? '' : 'p-invalid'"/>
            <small v-if="!isFactorioArchivePathValid" class="p-error">
              no such file on the server
            </small>
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
      <!--
        No "Enable REST API" checkbox any more: the REST API is the server that
        served this page, so it cannot be switched off from here. The port is
        persisted only and takes effect on the next `factorio-bot serve`.
      -->
      <div class="card p-fluid">
        <!--
          Relative, not `http://localhost:<the port field>`. This page was
          served by the API, so same-origin is right by construction, whereas
          the old absolute URL was wrong twice over a remote server: `localhost`
          named the viewer's machine, and the port came from an input the user
          may have just edited without restarting anything.
        -->
        <h5>REST API
          <a href="/swagger-ui/" target="_blank" rel="noopener noreferrer">swagger-ui</a>
          &middot;
          <a href="/openapi.json" target="_blank" rel="noopener noreferrer">openapi.json</a>
        </h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <div class="p-inputgroup">
              Port
              <InputText v-model="restapiPort" type="number" min="1" max="65535" />
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
            <InputText v-model="workspacePath" :class="isWorkspacePathValid ? '' : 'p-invalid'"/>
            <small v-if="!isWorkspacePathValid" class="p-error">
              no such directory on the server
            </small>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>

</style>
