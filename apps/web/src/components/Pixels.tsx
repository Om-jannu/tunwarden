import type { PixelArt } from "@/art/pixels";

// Draws generated pixel art. `scale` is whole pixels per grid cell, so the art
// stays on its grid; CSS may still size it, in whole multiples only.
export function Pixels({
  art,
  scale,
  className,
  label,
  children,
}: {
  art: PixelArt;
  scale: number;
  className?: string;
  label?: string;
  children?: React.ReactNode;
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
        <path key={fill} fill={fill} d={d} />
      ))}
      {children}
    </svg>
  );
}
