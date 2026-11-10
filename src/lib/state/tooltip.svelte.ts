// One global tooltip: any element with `data-tip="…"` gets it. The first tooltip waits 500 ms;
// peers hovered within 400 ms of the last one show instantly (Vercel guideline: delay the first,
// not the group). Optional `data-tip-kbd` renders a shortcut beside the text. Keyboard focus shows
// it too (after 300 ms): icon-only controls have no other label on screen.

class TipState {
  text = $state('');
  kbd = $state('');
  rect = $state<DOMRect | null>(null);
  visible = $state(false);
}

export const tip = new TipState();

export function installTooltips() {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let current: HTMLElement | null = null;
  let lastHidden = 0;

  const show = (el: HTMLElement) => {
    tip.text = el.dataset.tip ?? '';
    tip.kbd = el.dataset.tipKbd ?? '';
    tip.rect = el.getBoundingClientRect();
    tip.visible = !!tip.text;
  };
  const hide = () => {
    clearTimeout(timer);
    if (tip.visible) lastHidden = Date.now();
    tip.visible = false;
    current = null;
  };

  const over = (e: PointerEvent) => {
    const el = (e.target as HTMLElement | null)?.closest?.('[data-tip]') as HTMLElement | null;
    if (el === current) return;
    clearTimeout(timer);
    if (!el || document.body.dataset.dragging) {
      hide();
      return;
    }
    current = el;
    const warm = tip.visible || Date.now() - lastHidden < 400;
    if (warm) show(el);
    else {
      tip.visible = false;
      timer = setTimeout(() => current === el && show(el), 500);
    }
  };

  // The capture keydown below hides the old tip before Tab moves focus, so this shows the new one.
  const focusIn = (e: FocusEvent) => {
    const el = (e.target as HTMLElement | null)?.closest?.('[data-tip]') as HTMLElement | null;
    if (!el || !el.matches(':focus-visible')) return;
    clearTimeout(timer);
    current = el;
    timer = setTimeout(() => current === el && show(el), 300);
  };
  // Not at once: Chromium fires focusout while Svelte removes the focused element, in the middle of
  // its own update, where writing state throws. Focus that went straight to another tip keeps it.
  const focusOut = () => queueMicrotask(() => document.activeElement?.closest('[data-tip]') === current || hide());

  document.addEventListener('pointerover', over);
  document.addEventListener('focusin', focusIn);
  document.addEventListener('focusout', focusOut);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('keydown', hide, true);
  document.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
  return () => {
    document.removeEventListener('pointerover', over);
    document.removeEventListener('focusin', focusIn);
    document.removeEventListener('focusout', focusOut);
    document.removeEventListener('pointerdown', hide, true);
    document.removeEventListener('keydown', hide, true);
    document.removeEventListener('scroll', hide, true);
    window.removeEventListener('blur', hide);
  };
}
