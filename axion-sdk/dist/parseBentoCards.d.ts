import type { MissionState, BentoCard } from './types';
/**
 * Parses a MissionState into an ordered array of BentoCards.
 *
 * suggested_widgets format: "WidgetType:data_payload_key"
 * e.g. ["MetricCard:cheapest_flight_usd", "ChartCard:hotels", "ImageCard:rome_photo"]
 *
 * Falls back to inferring widgets from data_payload keys when suggested_widgets is absent.
 */
export declare function parseBentoCards(state: MissionState, options?: {
    refinedKeys?: Set<string>;
}): BentoCard[];
