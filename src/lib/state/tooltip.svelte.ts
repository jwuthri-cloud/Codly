// One global tooltip: any element with `data-tip="…"` gets it. The first tooltip waits 500 ms;
// peers hovered within 400 ms of the last one show instantly (Vercel guideline: delay the first,
// not the group). Optional `data-tip-kbd` renders a shortcut beside the text.

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

  document.addEventListener('pointerover', over);
  document.addEventListener('pointerdown', hide, true);
  document.addEventListener('keydown', hide, true);
  document.addEventListener('scroll', hide, true);
  window.addEventListener('blur', hide);
  return () => {
    document.removeEventListener('pointerover', over);
    document.removeEventListener('pointerdown', hide, true);
    document.removeEventListener('keydown', hide, true);
    document.removeEventListener('scroll', hide, true);
    window.removeEventListener('blur', hide);
  };
}
