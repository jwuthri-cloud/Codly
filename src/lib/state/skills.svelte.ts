// Open/closed state of the skills manager. Kept out of ui.svelte.ts so the feature stays in its own files.
class SkillsUi {
  open = $state(false);
  tab = $state<'installed' | 'browse'>('installed');
  show(tab: 'installed' | 'browse' = 'installed') {
    this.tab = tab;
    this.open = true;
  }
}
export const skillsUi = new SkillsUi();
