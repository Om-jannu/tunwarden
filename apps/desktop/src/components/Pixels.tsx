import type { PixelArt } from "@/art/pixels";

// Draws generated brand pixel art (docs/brand/export.py) at a whole-number
// scale, so it stays on its grid. Paths coloured "lamp" or "halo" take the
// colours given here: the state the lamp shows.
export function Pixels({
  art,
  scale,
  lamp = "#ffe8a3",
  halo = "#6b5f3a",
  className,
  label,
}: {
  art: PixelArt;
  scale: number;
  lamp?: string;
  halo?: string;
  className?: string;
  label?: string;
}) {
  return (
    <svg
      className={className}
      width={art.width * scale}
      height={art.height * scale}
      viewBox={`${art.x} ${art.y} ${art.width} ${art.height}`}
      shapeRendering="crispEdges"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {art.paths.map(([fill, d]) => (
        <path
          key={fill}
          fill={fill === "lamp" ? lamp : fill === "halo" ? halo : fill}
          d={d}
          // The lamp eases to its new aspect, like a signal changing.
          style={fill === "lamp" || fill === "halo" ? { transition: "fill 400ms cubic-bezier(0.16, 1, 0.3, 1)" } : undefined}
        />
      ))}
    </svg>
  );
}

/** The tray's lamp colours, by dashboard health. */
export const LAMP: Record<"idle" | "good" | "busy" | "bad", string> = {
  idle: "#8c96a5",
  good: "#3cc27a",
  busy: "#e3a43b",
  bad: "#f0645a",
};
