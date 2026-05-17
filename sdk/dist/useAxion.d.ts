import type { UseAxionOptions, UseAxionResult } from "./types";
export declare function useAxion<T = Record<string, unknown>>(intent: string, options?: UseAxionOptions): UseAxionResult<T>;
