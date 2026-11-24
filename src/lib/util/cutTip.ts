// A text that its box cuts (an ellipsis, a last line that does not show) gives its whole text
// as a tooltip (rooms/RoomHead.svelte). Pure: the box's sizes are read by the caller.

/** The sizes of a box and of what is in it, in px (an element's own fields). */
export type Fit = { scrollWidth: number; clientWidth: number; scrollHeight: number; clientHeight: number };

/** The tooltip of a text in `box`: the whole text when the box cuts it, null when it shows whole. */
export function cutTip(box: Fit, text: string): string | null {
  const cut = box.scrollWidth > box.clientWidth || box.scrollHeight > box.clientHeight;
  return cut && text ? text : null;
}
