// What the map calls the ready-made designs. Prices themselves come from src/office/price.ts
// (the core's mirror: one table, and the core charges).

/** How a ready-made design is called on the map. */
export const TEMPLATE_LABEL: Record<string, string> = { garage: 'Garage', 'small-office': 'Small office', studio: 'Studio' };
export const templateLabel = (t: string): string => TEMPLATE_LABEL[t] ?? t.replace(/-/g, ' ').replace(/^./, (c) => c.toUpperCase());
