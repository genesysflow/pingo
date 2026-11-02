import { mount, unmount } from 'svelte';
import App from './App.svelte';
import ServerSetup from './lib/desktop/ServerSetup.svelte';
import { nativeDesktop, loadDesktopPreferences, choosingServer, finishChoosingServer } from './lib/desktop/bridge';
import './app.css';
import { runVideoProbe } from './lib/desktop/probe';
async function start() {
  if (nativeDesktop && await runVideoProbe()) return;
  const target = document.getElementById('app')!;
  const current = nativeDesktop ? (await loadDesktopPreferences()).serverUrl : null;
  if (nativeDesktop && (!current || choosingServer())) {
    const setup = mount(ServerSetup, { target, props: { current, ready: () => {
      finishChoosingServer();
      void unmount(setup).then(() => mount(App, { target }));
    } } });
  } else mount(App, { target });
}
void start().catch(console.error);
