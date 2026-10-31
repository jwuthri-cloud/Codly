// A wall clock in the studio that keeps your real time (the office's day and night follow it).

import { BoxGeometry, CircleGeometry, CylinderGeometry, Group, Mesh, MeshStandardMaterial } from 'three';
import { canvas, tex } from './textures';

const R = 0.27;

export class WallClock {
  readonly group = new Group();
  private hourHand: Mesh;
  private minuteHand: Mesh;

  /** Hung at (x, y, z), its face turned to `rot` (like layout items). */
  constructor(x: number, y: number, z: number, rot: number) {
    const rim = new Mesh(new CylinderGeometry(R + 0.03, R + 0.03, 0.05, 40).rotateX(Math.PI / 2), new MeshStandardMaterial({ color: '#2b2622', roughness: 0.45, metalness: 0.3 }));
    const face = new Mesh(new CircleGeometry(R, 40), new MeshStandardMaterial({ map: tex(paintFace()), roughness: 0.6 }));
    face.position.z = 0.026;
    const ink = new MeshStandardMaterial({ color: '#1f1b18', roughness: 0.5 });
    const hand = (len: number, w: number, z: number) => {
      const m = new Mesh(new BoxGeometry(w, len + 0.05, 0.006).translate(0, len / 2 - 0.025, 0), ink);
      m.position.z = z;
      return m;
    };
    this.hourHand = hand(R * 0.52, 0.022, 0.032);
    this.minuteHand = hand(R * 0.8, 0.014, 0.038);
    const pin = new Mesh(new CylinderGeometry(0.014, 0.014, 0.012, 12).rotateX(Math.PI / 2), new MeshStandardMaterial({ color: '#c8553d' }));
    pin.position.z = 0.042;
    this.group.add(rim, face, this.hourHand, this.minuteHand, pin);
    for (const m of this.group.children) (m as Mesh).castShadow = true;
    this.group.position.set(x, y, z);
    this.group.rotation.y = rot;
  }

  /** Point the hands at this time. */
  set(d: Date) {
    const m = d.getMinutes() + d.getSeconds() / 60;
    this.minuteHand.rotation.z = (-m / 60) * Math.PI * 2;
    this.hourHand.rotation.z = (-((d.getHours() % 12) + m / 60) / 12) * Math.PI * 2;
  }
}

function paintFace(): HTMLCanvasElement {
  const [c, g] = canvas(256, 256);
  g.fillStyle = '#f6f1e6';
  g.fillRect(0, 0, 256, 256);
  g.translate(128, 128);
  g.fillStyle = '#2b2622';
  for (let i = 0; i < 60; i++) {
    const big = i % 5 === 0;
    g.save();
    g.rotate((i / 60) * Math.PI * 2);
    g.fillRect(big ? -3 : -1, -122, big ? 6 : 2, big ? 18 : 8);
    g.restore();
  }
  g.font = '600 30px system-ui, sans-serif';
  g.textAlign = 'center';
  g.textBaseline = 'middle';
  for (const [n, a] of [[12, 0], [3, 0.25], [6, 0.5], [9, 0.75]] as const) {
    const t = a * Math.PI * 2;
    g.fillText(String(n), Math.sin(t) * 82, -Math.cos(t) * 82);
  }
  return c;
}
