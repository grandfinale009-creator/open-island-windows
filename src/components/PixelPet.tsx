/**
 * PixelPet ¡ª Original pixel owl design for Open Island.
 * 7¡Á5 pixel owl face in 13¡Á8 viewBox, crisp edges, drop-shadow glow per state.
 *
 * Original design ¡ª NOT ported from vibeisland.app or any other source.
 * Owl symbolises wisdom ¡ª fitting for a coding agent companion.
 *
 * Layout (7 wide ¡Á 5 tall, positioned to match original cat proportions):
 *   Row y=1: ear tufts (bright tips)
 *   Row y=2: 2¡Á2 eye sockets (bright) + forehead
 *   Row y=3: 2¡Á2 pupils (black) + beak (dark)
 *   Row y=4: body
 *   Row y=5: feet
 *
 * States:
 *   idle    ¡ª normal eyes, relaxed (green)
 *   working ¡ª focused expression (blue)
 *   waiting ¡ª alert wide eyes (orange)
 */

export type PetState = "idle" | "working" | "waiting";

interface PixelPetProps {
  state: PetState;
  size?: number;
}

const PALETTES: Record<PetState, { fill: string; bright: string; dark: string }> = {
  idle:    { fill: "#22c55e", bright: "#4ade80", dark: "#16a34a" },
  working: { fill: "#3b82f6", bright: "#60a5fa", dark: "#2563eb" },
  waiting: { fill: "#f97316", bright: "#fb923c", dark: "#ea580c" },
};

export function PixelPet({ state, size = 26 }: PixelPetProps) {
  const { fill, bright, dark } = PALETTES[state];

  return (
    <svg
      width={size}
      height={size * (16 / 26)}
      viewBox="0 0 13 8"
      shapeRendering="crispEdges"
      style={{ filter: `drop-shadow(0 0 3px ${fill})` }}
    >
      {/* === Row y=1: Ear tufts (bright tips) === */}
      <rect x="3" y="1" width="1" height="1" fill={bright} />
      <rect x="9" y="1" width="1" height="1" fill={bright} />

      {/* === Row y=2: Eye sockets (2¡Á2 bright) + forehead === */}
      <rect x="3" y="2" width="3" height="1" fill={fill} />
      <rect x="7" y="2" width="3" height="1" fill={fill} />
      <rect x="4" y="2" width="2" height="1" fill={bright} />
      <rect x="8" y="2" width="2" height="1" fill={bright} />

      {/* === Row y=3: Pupils (2¡Á2 black) + beak === */}
      <rect x="3" y="3" width="3" height="1" fill={fill} />
      <rect x="7" y="3" width="3" height="1" fill={fill} />
      <rect x="4" y="3" width="2" height="1" fill="#000" />
      <rect x="8" y="3" width="2" height="1" fill="#000" />
      {/* Beak ¡ª centered, dark */}
      <rect x="6" y="3" width="1" height="1" fill={dark} />

      {/* === Row y=4: Body === */}
      <rect x="3" y="4" width="7" height="1" fill={dark} />

      {/* === Row y=5: Feet === */}
      <rect x="4" y="5" width="2" height="1" fill={fill} />
      <rect x="7" y="5" width="2" height="1" fill={fill} />
    </svg>
  );
}

export function getPetState(running: number, waiting: number): PetState {
  if (waiting > 0) return "waiting";
  if (running > 0) return "working";
  return "idle";
}