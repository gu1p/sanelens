import { LIST_SEPARATOR } from "./constants";
import type { LogEvent, PanelState } from "./types";

export function normalizeFilterToken(value: string): string {
  if (!value) {
    return "";
  }
  return value.trim().toLowerCase();
}

export function normalizeServiceToken(value: string): string {
  if (!value) {
    return "";
  }
  return value.trim();
}

export function encodeToken(value: string): string {
  return encodeURIComponent(value).replace(/~/g, "%7E");
}

export function decodeToken(value: string): string {
  if (!value) {
    return "";
  }
  const sanitized = value.replace(/\+/g, " ");
  try {
    return decodeURIComponent(sanitized);
  } catch {
    return sanitized;
  }
}

export function encodeTokenList(tokens: string[]): string {
  return tokens.map((token) => encodeToken(token)).join(LIST_SEPARATOR);
}

export function decodeTokenList(
  value: string,
  normalizer: (value: string) => string = normalizeServiceToken
): string[] {
  if (!value) {
    return [];
  }
  return value
    .split(LIST_SEPARATOR)
    .map((token) => decodeToken(token))
    .map((token) => normalizer(token))
    .filter(Boolean);
}

export function buildPanelMeta(panel: PanelState): string {
  let label = "ALL SERVICES";
  if (panel.filter && panel.filter.length === 1) {
    label = panel.filter[0].toUpperCase();
  } else if (panel.filter && panel.filter.length > 1) {
    label = `${panel.filter.length} SERVICES`;
  }
  const includeCount = panel.include.length;
  const excludeCount = panel.exclude.length;
  const parts = [label];
  if (includeCount) {
    parts.push(`+${includeCount} include`);
  }
  if (excludeCount) {
    parts.push(`-${excludeCount} exclude`);
  }
  return parts.join(" | ");
}

export function entryMatchesPanel(panel: PanelState, entry: LogEvent): boolean {
  if (panel.filter && !panel.filter.includes(entry.service)) {
    return false;
  }
  if (panel.include.length === 0 && panel.exclude.length === 0) {
    return true;
  }
  const normalizedLine = String(entry.line).toLowerCase();
  if (panel.include.length && !panel.include.some((token) => normalizedLine.includes(token))) {
    return false;
  }
  if (panel.exclude.length && panel.exclude.some((token) => normalizedLine.includes(token))) {
    return false;
  }
  return true;
}
