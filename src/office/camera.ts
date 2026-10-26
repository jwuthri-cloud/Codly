// Camera rig: an isometric-ish 3/4 view of the office diorama. Pan, zoom and a constrained
// orbit (a little yaw, a narrow pitch band — it always reads as "seen from above at an angle")
// with damping, smooth fly-to and an optional follow target. Frame-rate independent.

import { PerspectiveCamera, Plane, Raycaster, Vector2, Vector3 } from 'three';

export interface View {
  x: number;
  y: number;
  z: number;
  yaw: number;
  pitch: number;
  dist: number;
}

const clamp = (v: number, a: number, b: number) => Math.min(b, Math.max(a, v));

export const DEFAULT_YAW = 0.6;
export const DEFAULT_PITCH = 0.84;

export class CameraRig {
  camera: PerspectiveCamera;
  cur: View = { x: 0, y: 0, z: 0, yaw: DEFAULT_YAW, pitch: DEFAULT_PITCH, dist: 40 };
  goal: View = { ...this.cur };
  /** Smoothing rate (1/s): high while the user drives, lower for fly-to. */
  private rate = 9;
  follow: (() => Vector3 | null) | null = null;
  /** Target bounds (the floor, centered on the origin). */
  boundsX = 16;
  boundsZ = 14;
  minDist = 7;
  maxDist = 70;
  yawMin = DEFAULT_YAW - 0.75;
  yawMax = DEFAULT_YAW + 0.75;
  pitchMin = 0.62;
  pitchMax = 1.22;
  private ground = new Plane(new Vector3(0, 1, 0), 0);
  private ray = new Raycaster();
  private ndc = new Vector2();

  constructor(aspect: number) {
    this.camera = new PerspectiveCamera(30, aspect, 0.3, 400);
    this.apply();
  }

  setAspect(a: number) {
    this.camera.aspect = a;
    this.camera.updateProjectionMatrix();
  }

  /** Center the projection on the part of the view not covered by app UI. */
  setFreeArea(W: number, H: number, insets: { top: number; right: number; bottom: number; left: number }) {
    const ox = (insets.right - insets.left) / 2;
    const oy = (insets.bottom - insets.top) / 2;
    if (Math.abs(ox) < 0.5 && Math.abs(oy) < 0.5) this.camera.clearViewOffset();
    else this.camera.setViewOffset(W, H, ox, oy, W, H);
    this.camera.updateProjectionMatrix();
  }

  /** Jump (no animation). */
  set(v: Partial<View>) {
    Object.assign(this.goal, v);
    this.clampGoal();
    this.cur = { ...this.goal };
    this.apply();
  }

  flyTo(v: Partial<View>, instant = false) {
    Object.assign(this.goal, v);
    this.clampGoal();
    this.rate = 3.4;
    if (instant) {
      this.cur = { ...this.goal };
      this.apply();
    }
  }

  orbit(dxPx: number, dyPx: number) {
    this.rate = 12;
    this.goal.yaw = clamp(this.goal.yaw - dxPx * 0.005, this.yawMin, this.yawMax);
    this.goal.pitch = clamp(this.goal.pitch + dyPx * 0.004, this.pitchMin, this.pitchMax);
  }

  /** Pan by screen pixels (ground-plane motion scaled by distance). */
  pan(dxPx: number, dyPx: number, viewH: number) {
    this.rate = 14;
    const k = (this.goal.dist * Math.tan((this.camera.fov * Math.PI) / 360) * 2) / Math.max(1, viewH);
    const s = Math.sin(this.goal.yaw);
    const c = Math.cos(this.goal.yaw);
    // Screen right = (c, −s) on the ground; screen up = (−s, −c), stretched by 1/sin(pitch).
    const up = 1 / Math.max(0.35, Math.sin(this.goal.pitch));
    this.goal.x -= (dxPx * c + dyPx * s * up) * k;
    this.goal.z -= (-dxPx * s + dyPx * c * up) * k;
    this.follow = null;
    this.clampGoal();
  }

  /** Zoom by a factor (>1 = closer), toward a ground point when given. */
  zoom(factor: number, anchor?: Vector3 | null) {
    this.rate = 12;
    const next = clamp(this.goal.dist / factor, this.minDist, this.maxDist);
    const real = this.goal.dist / next;
    if (anchor && !this.follow) {
      const t = 1 - 1 / real;
      this.goal.x += (anchor.x - this.goal.x) * t;
      this.goal.z += (anchor.z - this.goal.z) * t;
    }
    this.goal.dist = next;
    this.clampGoal();
  }

  private clampGoal() {
    this.goal.x = clamp(this.goal.x, -this.boundsX, this.boundsX);
    this.goal.z = clamp(this.goal.z, -this.boundsZ, this.boundsZ);
    this.goal.dist = clamp(this.goal.dist, this.minDist, this.maxDist);
    this.goal.pitch = clamp(this.goal.pitch, this.pitchMin, this.pitchMax);
    this.goal.yaw = clamp(this.goal.yaw, this.yawMin, this.yawMax);
  }

  /** Ground point under a screen position (NDC −1..1). */
  groundAt(nx: number, ny: number): Vector3 | null {
    this.ndc.set(nx, ny);
    this.ray.setFromCamera(this.ndc, this.camera);
    const p = new Vector3();
    return this.ray.ray.intersectPlane(this.ground, p) ? p : null;
  }

  raycaster(nx: number, ny: number): Raycaster {
    this.ndc.set(nx, ny);
    this.ray.setFromCamera(this.ndc, this.camera);
    return this.ray;
  }

  /**
   * A view at `yaw`/`pitch` that frames `points` with a margin (NDC fraction). `availX` /
   * `availY`: the fraction of the view not covered by app UI (the camera's view offset already
   * centers the projection on that free area).
   */
  fit(points: Vector3[], yaw: number, pitch: number, margin = 0.06, availX = 1, availY = 1): View {
    const cam = this.camera.clone();
    let cx = 0;
    let cz = 0;
    for (const p of points) {
      cx += p.x;
      cz += p.z;
    }
    const v: View = { x: cx / Math.max(1, points.length), y: 0, z: cz / Math.max(1, points.length), yaw, pitch, dist: 40 };
    const fov = (cam.fov * Math.PI) / 180;
    const q = new Vector3();
    for (let i = 0; i < 10; i++) {
      const c = Math.cos(pitch);
      cam.position.set(v.x + Math.sin(yaw) * c * v.dist, Math.sin(pitch) * v.dist, v.z + Math.cos(yaw) * c * v.dist);
      cam.lookAt(v.x, 0, v.z);
      cam.updateMatrixWorld();
      let x0 = Infinity;
      let x1 = -Infinity;
      let y0 = Infinity;
      let y1 = -Infinity;
      for (const p of points) {
        q.copy(p).project(cam);
        x0 = Math.min(x0, q.x);
        x1 = Math.max(x1, q.x);
        y0 = Math.min(y0, q.y);
        y1 = Math.max(y1, q.y);
      }
      const mx = (x0 + x1) / 2;
      const my = (y0 + y1) / 2;
      const halfH = v.dist * Math.tan(fov / 2);
      const halfW = halfH * cam.aspect;
      const rx = Math.cos(yaw);
      const rz = -Math.sin(yaw);
      const fx = -Math.sin(yaw);
      const fz = -Math.cos(yaw);
      const up = 1 / Math.max(0.3, Math.sin(pitch));
      v.x += rx * mx * halfW + fx * my * halfH * up * 0.9;
      v.z += rz * mx * halfW + fz * my * halfH * up * 0.9;
      const s = Math.max((x1 - x0) / 2 / availX / (1 - margin), (y1 - y0) / 2 / availY / (1 - margin));
      v.dist = clamp(v.dist * (0.35 + 0.65 * s), this.minDist, this.maxDist);
    }
    return v;
  }

  /** Advance damping; true while moving. */
  update(dt: number): boolean {
    if (this.follow) {
      const p = this.follow();
      if (p) {
        this.goal.x = clamp(p.x, -this.boundsX, this.boundsX);
        this.goal.z = clamp(p.z, -this.boundsZ, this.boundsZ);
      }
    }
    const k = 1 - Math.exp(-dt * this.rate);
    let moving = false;
    for (const key of ['x', 'y', 'z', 'pitch', 'dist', 'yaw'] as const) {
      const d = this.goal[key] - this.cur[key];
      const eps = key === 'dist' ? 0.004 : key === 'pitch' || key === 'yaw' ? 0.0003 : 0.002;
      if (Math.abs(d) > eps) {
        this.cur[key] += d * k;
        moving = true;
      } else this.cur[key] = this.goal[key];
    }
    this.apply();
    return moving;
  }

  private apply() {
    const v = this.cur;
    const cp = Math.cos(v.pitch);
    this.camera.position.set(v.x + Math.sin(v.yaw) * cp * v.dist, v.y + Math.sin(v.pitch) * v.dist, v.z + Math.cos(v.yaw) * cp * v.dist);
    this.camera.lookAt(v.x, v.y, v.z);
    this.camera.updateMatrixWorld();
  }
}
