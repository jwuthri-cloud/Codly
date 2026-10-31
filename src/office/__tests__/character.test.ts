// Every hairstyle on every role builds clean geometry (no NaN from the sculpting math).
import { Mesh, Texture } from 'three';
import { expect, it } from 'vitest';
import { Character, LOOK_STYLES, lookFor, PropKit } from '../gfx/character';

it('builds every look without NaN', () => {
  const env = { emotes: () => new Texture(), props: new PropKit() };
  const kinds = [['lead', 'none', false], ['reviewer', 'none', false], ['worker', 'hood', false], ['worker', 'headphones', false], ['solo', 'none', false], ['worker', 'none', true]] as const;
  for (const [role, acc, intern] of kinds)
    for (let style = 0; style < LOOK_STYLES; style++) {
      const c = new Character('x', lookFor(`x#style=${style}&acc=${acc}`, 3, role, 'claude', intern), env);
      c.root.traverse((o) => {
        if (o instanceof Mesh) expect(Number.isFinite(o.geometry.boundingSphere?.radius ?? 0), `${role} ${acc} ${style}`).toBe(true);
      });
      c.dispose();
    }
});
