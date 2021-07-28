<template>
	<div class="p-grid">
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
              <InputText placeholder="Workspace Folder"/>
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
            <Dropdown id="factorio_version" v-model="factorioVersion" :options="availableFactorioVersion" optionLabel="name" placeholder="Select One"></Dropdown>
          </div>
        </div>
      </div>
      <div class="card p-fluid">
        <h5>Number of Factorio Client Instances</h5>
        <div class="p-formgrid p-grid">
          <div class="p-field p-col">
            <label for="client_count">Client Instances: {{ clientCount }}</label>
            <Slider id="client_count" v-model="clientCount" :min="0" :max="16" />
          </div>
        </div>
      </div>




    </div>
	</div>
</template>

<script>
export default {

  data() {
    return {
      availableFactorioVersion: [
        {name: 'Option 1', code: 'Option 1'},
        {name: 'Option 2', code: 'Option 2'},
        {name: 'Option 3', code: 'Option 3'}
      ],
      factorioVersion: null,
      clientCount: 2,
    }
  }
}
</script>

<style scoped>

</style>
