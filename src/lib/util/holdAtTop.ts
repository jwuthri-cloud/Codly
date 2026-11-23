// For a link that opens a long, scrolling page at one of its sections. One scroll is not enough:
// a section above that shows its rows a moment later (it reads the repo first) pushes this one
// down again, and WebKit, the desktop app's engine, does not keep the place as Chrome does.

/** The first wheel, key or press means the scroll is yours. */
const YOURS = ['wheel', 'keydown', 'pointerdown'];

/**
 * Scrolls `el` to the top of its scroller and puts it back each time a section beside it changes
 * height, appears or goes, until your first wheel, key or press. `el` is a direct child of the
 * scrolling content. Returns the function that lets go (call it when `el` goes away).
 */
export function holdAtTop(el: HTMLElement): () => void {
  const content = el.parentElement;
  if (!content) throw new Error('holdAtTop: the element is not in the page');
  const place = () => el.scrollIntoView({ block: 'start' });
  const sizes = new ResizeObserver(place);
  const watch = () => {
    for (const section of content.children) sizes.observe(section);
  };
  const sections = new MutationObserver(() => {
    watch();
    place();
  });
  const ended = new AbortController();
  const letGo = () => {
    sizes.disconnect();
    sections.disconnect();
    ended.abort();
  };
  place();
  watch();
  sections.observe(content, { childList: true });
  for (const type of YOURS) window.addEventListener(type, letGo, { capture: true, passive: true, signal: ended.signal });
  return letGo;
}
