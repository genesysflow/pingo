import { mount, unmount } from 'svelte';
import App from './App.svelte';
import ServerSetup from './lib/desktop/ServerSetup.svelte';
import { nativeDesktop, loadDesktopPreferences, choosingServer, finishChoosingServer, checkServer, incompatibility } from './lib/desktop/bridge';
import './app.css';
import { runVideoProbe } from './lib/desktop/probe';
async function start() {
  if (nativeDesktop && await runVideoProbe()) return;
  const target = document.getElementById('app')!;
  const current = nativeDesktop ? (await loadDesktopPreferences()).serverUrl : null;
  const setup = (problem = '') => {
    const screen = mount(ServerSetup, { target, props: { current, problem, ready: () => {
      finishChoosingServer();
      // After a failed startup check the data layer already points at the old server; start clean.
      if (problem) location.reload();
      else void unmount(screen).then(() => mount(App, { target }));
    } } });
  };
  if (nativeDesktop && (!current || choosingServer())) return setup();
  const app = mount(App, { target });
  // The server may have been updated past this app. Check without delaying startup; an unreachable server
  // is left to the app's own connection handling.
  if (nativeDesktop && current)
    void checkServer(current).then(async (check) => {
      const problem = incompatibility(check);
      if (!problem) return;
      await unmount(app);
      setup(problem);
    }, () => {});
}
void start().catch(console.error);
