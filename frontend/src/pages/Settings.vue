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
              <InputText v-model="workspacePath"/>
              <Button label="Select"/>
            </div>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Factorio Version</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="factorio_version">Version</label>
            <Dropdown id="factorio_version" v-model="factorioVersion" :options="availableFactorioVersions" optionLabel="name" placeholder="Select One"></Dropdown>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Number of Factorio Client Instances</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="client_count">Client Instances: {{ settings.client_count }}</label>
            <Slider id="client_count" v-model="clientCount" :min="0" :max="16" />
          </div>
        </div>
      </div>
    </div>
	</div>
</template>

<script>
import {computed, defineComponent, ref} from 'vue';
import {useAppStore} from '@/store/appStore';
import {useFactorioVersionsStore} from '@/store/factorioVersionsStore';

export default defineComponent({
  setup(props, {emit}) {
    const appStore = useAppStore();
    const factorioVersionsStore = useFactorioVersionsStore();
    factorioVersionsStore.loadFactorioVersions()

    const workspacePath = computed({
      get() {
        return appStore.settings.workspace_path
      },
      set(val) {
        appStore.updateWorkspacePath(val)
      }
    })

    const factorioVersion = computed({
      get() {
        const v = appStore.settings.factorio_version
        return {code: v, name: v}
      },
      set(val) {
        appStore.updateFactorioVersion(val.code)
      }
    })
    const clientCount = computed({
      get() {
        return appStore.settings.client_count
      },
      set(val) {
        appStore.updateClientCount(val)
      }
    })

    return {
      factorioVersion,
      workspacePath,
      clientCount,
      settings: computed(() => appStore.getSettings),
      availableFactorioVersions: computed(() => factorioVersionsStore.getFactorioVersions.map(version => ({
        name: version, code: version
      }))),
      onMenuToggle: function(event) {
        emit('menu-toggle', event);
      }
    }
  }
});
</script>

<style scoped>

</style>
