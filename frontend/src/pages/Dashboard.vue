<template>
<div class="p-grid p-fluid dashboard">
	<div class="p-col-12 p-lg-4">
		<div class="card summary">
			<span class="title">Instances</span>
			<span class="detail">Number of configured instances</span>
			<span class="count visitors" @click="sendTestMessage()">2</span>
		</div>
	</div>
	<div class="p-col-12 p-lg-4">
		<div class="card summary">
			<span class="title">Sales</span>
			<span class="detail">Number of purchases</span>
			<span class="count purchases">534</span>
		</div>
	</div>
	<div class="p-col-12 p-lg-4">
		<div class="card summary">
			<span class="title">Revenue</span>
			<span class="detail">Income for today</span>
			<span class="count revenue">$3,200</span>
		</div>
	</div>
</div>
</template>

<script>
import {defineComponent} from "vue";
import { listen } from '@tauri-apps/api/event'

import { invoke } from '@tauri-apps/api/tauri'
export default defineComponent({
  components: {
  },
  setup() {
    return {
      sendTestMessage: async() => {
        console.log('listen to the_event');
        await listen('the_event', event => {
          console.log('event from rust', event)
          // event.event is the event name (useful if you want to use a single callback fn for multiple event types)
          // event.payload is the payload object
        })
        await invoke('my_custom_command')
        /*
        // listen to the `click` event and get a function to remove the event listener
        // there's also a `once` function that subscribes to an event and automatically unsubscribes the listener on the first event


        // emits the `click` event with the object payload
        emit('click', {
          theMessage: 'Tauri is awesome!'
        })
         */

      }
    }
  },
})
</script>
