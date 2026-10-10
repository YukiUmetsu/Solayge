import { useMemo } from "react";

import qrcode from "qrcode-generator";

const QUIET_ZONE = 4;

/**
 * Render a QR code as an inline SVG built from the module matrix.
 *
 * Uses the zero-dependency `qrcode-generator` encoder and draws rectangles
 * directly (no canvas, no innerHTML), so it is safe and testable in jsdom.
 */
export function QrCode({
  value,
  size = 240,
  label = "Pairing QR code",
}: {
  value: string;
  size?: number;
  label?: string;
}) {
  const { count, cells } = useMemo(() => {
    const qr = qrcode(0, "M");
    qr.addData(value, "Byte");
    qr.make();
    const count = qr.getModuleCount();
    const cells: boolean[] = [];
    for (let row = 0; row < count; row += 1) {
      for (let col = 0; col < count; col += 1) {
        cells.push(qr.isDark(row, col));
      }
    }
    return { count, cells };
  }, [value]);

  const total = count + QUIET_ZONE * 2;
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${total} ${total}`}
      role="img"
      aria-label={label}
      shapeRendering="crispEdges"
      className="rounded-lg border border-line bg-white"
    >
      <rect width={total} height={total} fill="#ffffff" />
      {cells.map((dark, index) =>
        dark ? (
          <rect
            key={index}
            x={(index % count) + QUIET_ZONE}
            y={Math.floor(index / count) + QUIET_ZONE}
            width={1}
            height={1}
            fill="#000000"
          />
        ) : null,
      )}
    </svg>
  );
}
