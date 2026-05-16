export const MODEL_CATALOG = [
  {
    id:          "gpt-4o-mini",
    label:       "GPT-4o mini",
    description: "Fast, cheap, great for most missions",
    inputPer1M:  0.15,
    outputPer1M: 0.60,
    badge:       "Default" as string | null,
  },
  {
    id:          "gpt-4o",
    label:       "GPT-4o",
    description: "Best quality, handles complex reasoning",
    inputPer1M:  2.50,
    outputPer1M: 10.00,
    badge:       "Recommended" as string | null,
  },
  {
    id:          "o3-mini",
    label:       "o3-mini",
    description: "Strong reasoning model, efficient for structured tasks",
    inputPer1M:  1.10,
    outputPer1M: 4.40,
    badge:       "Reasoning" as string | null,
  },
  {
    id:          "o1-mini",
    label:       "o1-mini",
    description: "Advanced reasoning, slower response time",
    inputPer1M:  3.00,
    outputPer1M: 12.00,
    badge:       null as string | null,
  },
] as const;

export type ModelId = "gpt-4o-mini" | "gpt-4o" | "o3-mini" | "o1-mini";
