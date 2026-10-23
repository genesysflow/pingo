export interface Point {
  x: number;
  y: number;
}
/** Positions use canvas units. Equal-power falloff avoids a hard edge at the hearing radius. */
export function spatialAudio(listener: Point, speaker: Point, range = 260): { volume: number; pan: number } {
  if (range <= 0 || ![listener.x, listener.y, speaker.x, speaker.y, range].every(Number.isFinite))
    return { volume: 0, pan: 0 };
  const dx = speaker.x - listener.x,
    distance = Math.hypot(dx, speaker.y - listener.y);
  return {
    volume: distance >= range ? 0 : Math.cos(((distance / range) * Math.PI) / 2) ** 2,
    pan: Math.max(-1, Math.min(1, dx / range)),
  };
}
