<template>
  <SplitButton label="Start" icon="pi pi-check" :model="items" class="p-button-success p-mr-2 p-mb-2"></SplitButton>
</template>

<script>
export default {

  data() {
    return {
      items: [
        {
          label: 'Update',
          icon: 'pi pi-refresh'
        },
        {
          label: 'Delete',
          icon: 'pi pi-times'
        },
        {
          separator:true
        },
        {
          label: 'Home',
          icon: 'pi pi-home'
        },
      ]
    }
  }
}
</script>

<style scoped>

</style>
