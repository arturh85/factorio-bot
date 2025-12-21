import {  } from '@tauri-apps/api';
import { YNetwork } from 'ynetwork';
import { fetch } from '@tauri-apps/plugin-http';

async function tauriReuestResolver({ method, url, data, headers }) {

  try {

    const response = await fetch(url, {
      method: method.toUpperCase(),
      body: data,
      headers,
      responseType: 2
    });

    let result = response.data;
    let status = response.status ?? -1;
    let responseHeaders = response.headers ?? {};

    if (response.headers['content-type']?.includes('application/json')) {
      result = JSON.parse(result.toString());
    }

    return {
      headers: responseHeaders,
      status,
      data: result
    };

  }
  catch (error) {

    return {
      status: -1,
      data: error.message,
      headers: {},
      error
    };

  }

}

console.log('doing stuff');

YNetwork.setDebug(import.meta.env.DEV);

if (import.meta.env.VITE_HTTP_HANDLER === 'NATIVE') {
  YNetwork.setRequestRunner(tauriReuestResolver);
}
