import type { MissionState, BentoCard } from './types';

/**
 * Parses a MissionState into an ordered array of BentoCards.
 *
 * suggested_widgets format: "WidgetType:data_payload_key"
 * e.g. ["MetricCard:cheapest_flight_usd", "ChartCard:hotels", "ImageCard:rome_photo"]
 *
 * Falls back to inferring widgets from data_payload keys when suggested_widgets is absent.
 */
export function parseBentoCards(
  state: MissionState,
  options?: { refinedKeys?: Set<string> }
): BentoCard[] {
  const payload = state.data_payload ?? {};
  const widgets = state.suggested_widgets ?? [];
  const refinedKeys = options?.refinedKeys ?? new Set<string>();

  if (widgets.length > 0) {
    return widgets
      .map((w): BentoCard | null => {
        const colonIdx = w.indexOf(':');
        if (colonIdx === -1) return null;
        const widget = w.slice(0, colonIdx);
        const key    = w.slice(colonIdx + 1);
        const raw    = payload[key];
        if (raw === undefined || raw === null) return null;

        let props: Record<string, unknown>;
        if (widget === 'MetricCard') {
          if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
            props = raw as Record<string, unknown>;
          } else {
            props = { value: raw, title: humanizeKey(key) };
          }
        } else if (widget === 'ImageCard') {
          if (typeof raw === 'string') {
            props = { description: raw, title: humanizeKey(key) };
          } else if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
            props = raw as Record<string, unknown>;
          } else {
            return null;
          }
        } else if (widget === 'ChartCard') {
          if (typeof raw === 'object' && raw !== null && !Array.isArray(raw)) {
            props = { title: humanizeKey(key), ...(raw as Record<string, unknown>) };
          } else if (Array.isArray(raw)) {
            props = { title: humanizeKey(key), data: raw };
          } else {
            return null;
          }
        } else {
          // ComparisonTable, Timeline, or unknown: pass raw as-is with title
          if (typeof raw === 'object' && raw !== null) {
            props = { title: humanizeKey(key), ...(raw as Record<string, unknown>) };
          } else {
            return null;
          }
        }

        return {
          key,
          widget,
          props,
          isRefined: refinedKeys.has(key),
        };
      })
      .filter((c): c is BentoCard => c !== null);
  }

  // Fallback: infer from payload keys (no suggested_widgets)
  return Object.entries(payload)
    .filter(([k]) => !k.startsWith('_'))
    .map(([key, raw]): BentoCard => ({
      key,
      widget: inferWidget(key, raw),
      props:  typeof raw === 'object' && raw !== null
                ? { title: humanizeKey(key), ...(raw as Record<string, unknown>) }
                : { value: raw, title: humanizeKey(key) },
    }));
}

function inferWidget(key: string, value: unknown): string {
  if (typeof value === 'number') return 'MetricCard';
  if (typeof value === 'string' && (value.startsWith('http://') || value.startsWith('https://'))) return 'ImageCard';
  if (Array.isArray(value) && value.length > 0 && typeof value[0] === 'object') return 'ComparisonTable';
  if (typeof value === 'string') return 'MetricCard';
  return 'MetricCard';
}

function humanizeKey(key: string): string {
  return key
    .replace(/_/g, ' ')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/\b(usd|eur|gbp|url|api|id)\b/gi, (m) => m.toUpperCase())
    .replace(/\b\w/g, (c) => c.toUpperCase())
    .trim();
}
