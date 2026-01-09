const palette = [
  "#e07a5f",
  "#3d405b",
  "#81b29a",
  "#f2cc8f",
  "#f4a261",
  "#2a9d8f",
  "#6d597a",
  "#f94144",
  "#8ecae6",
];

const serviceColors = new Map<string, string>();

export function colorFor(service: string): string {
  if (!serviceColors.has(service)) {
    const color = palette[serviceColors.size % palette.length];
    serviceColors.set(service, color);
  }
  return serviceColors.get(service) ?? palette[0];
}
