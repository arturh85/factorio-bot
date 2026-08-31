<script setup lang="ts">
import {computed, ref, watch} from 'vue';
import {useAppStore} from '@/store/appStore';
import Card from '@/components/ui/Card.vue';
import Label from '@/components/ui/Label.vue';
import Input from '@/components/ui/Input.vue';
import Checkbox from '@/components/ui/Checkbox.vue';
import Slider from '@/components/ui/Slider.vue';

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
// `getRecreateLevel` is `boolean | undefined` -- `undefined` before settings
// have loaded. Narrowed to `=== true` so this checkbox and ProcessControl.vue's
// Toggle (Task 5) agree on the unloaded state instead of one showing checked
// and the other unchecked for the same underlying setting.
const recreateLevel = computed({
  get(): boolean {
    return appStore.getRecreateLevel === true
  },
  set(val: boolean) {
    appStore.updateRecreateLevel(val)
  }
})
const enableAutostart = computed({
  get(): boolean {
    return appStore.getEnableAutostart === true
  },
  set(val: boolean) {
    appStore.updateEnableAutostart(val)
  }
})
const restapiPort = computed({
  get(): string {
    return String(appStore.getRestapiPort ?? '')
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
  <div v-if="settings" class="mx-auto max-w-3xl">
    <Card title="Settings">
      <p class="text-ink-muted">Everything below is stored on the server and applies to the Factorio instances it manages.</p>
    </Card>

    <Card>
      <template #title>
        Factorio Archive &mdash; download from
        <a class="text-link" href="https://factorio.com/download" target="_blank" rel="noopener noreferrer">factorio.com/download</a>
      </template>
      <Label for="factorio-archive-path">Archive path on the server</Label>
      <Input
        id="factorio-archive-path"
        v-model="factorioArchivePath"
        :invalid="!isFactorioArchivePathValid"
        data-testid="archive-input"/>
      <p v-if="!isFactorioArchivePathValid" class="mt-1 text-sm text-danger">no such file on the server</p>
    </Card>

    <Card title="Workspace Folder">
      <Label for="workspace-path">Directory path on the server</Label>
      <Input
        id="workspace-path"
        v-model="workspacePath"
        :invalid="!isWorkspacePathValid"
        data-testid="workspace-input"/>
      <p v-if="!isWorkspacePathValid" class="mt-1 text-sm text-danger">no such directory on the server</p>
    </Card>

    <Card title="Startup">
      <div class="flex flex-col gap-3 sm:flex-row sm:gap-8">
        <Checkbox v-model="recreateLevel" label="Recreate level on start" data-testid="recreate-checkbox"/>
        <Checkbox v-model="enableAutostart" label="Start Factorio automatically" data-testid="autostart-checkbox"/>
      </div>
    </Card>

    <Card title="HTTP API">
      <p class="mb-3 text-ink-muted">
        This page is served by the same server that exposes the API.
        <a class="text-link" href="/swagger-ui/" target="_blank" rel="noopener noreferrer">Swagger UI</a>
        &middot;
        <a class="text-link" href="/openapi.json" target="_blank" rel="noopener noreferrer">openapi.json</a>
      </p>
      <Label for="restapi-port">Port (applies on the next server start)</Label>
      <Input
        id="restapi-port"
        v-model="restapiPort"
        type="number"
        min="1"
        max="65535"
        data-testid="port-input"/>
    </Card>

    <Card title="Map Exchange String">
      <Label for="map-exchange-string">Pasted from Factorio's map generator</Label>
      <Input id="map-exchange-string" v-model="mapExchangeString" data-testid="map-exchange-input"/>
    </Card>

    <Card title="Seed">
      <Label for="seed">Map seed</Label>
      <Input id="seed" v-model="seed" data-testid="seed-input"/>
    </Card>

    <Card title="Factorio Client Instances">
      <Label data-testid="client-count-label">Client Instances: {{ clientCount }}</Label>
      <Slider v-model="clientCount" :min="0" :max="16" label="Client Instances"/>
    </Card>
  </div>
</template>
