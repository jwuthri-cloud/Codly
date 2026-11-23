// Opening Settings at a place (a tab id, or `safetyLink`). A closed sheet opens there; an open
// one goes there at once (a notification clicked while Settings is open), so the link is never
// left behind for the next time you open Settings.
import { ui } from '../lib/state/ui.svelte';
import { openAt, type SettingsTarget } from './settingsPages';

let follow: ((to: SettingsTarget) => void) | null = null;

/** The open sheet goes where each link says, with `go`, until the returned function is called. */
export function followLinks(go: (to: SettingsTarget) => void): () => void {
  follow = go;
  return () => {
    if (follow === go) follow = null;
  };
}

/** Settings at `link` (the format of `ui.settingsTab`). */
export function openSettings(link: string) {
  if (follow) follow(openAt(link));
  else ui.settingsTab = link;
  ui.settingsOpen = true;
}
