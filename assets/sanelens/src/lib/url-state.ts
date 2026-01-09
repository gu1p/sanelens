import {
  GROUP_SEPARATOR,
  PANEL_SEPARATOR,
  URL_ACTIVE_KEY,
  URL_STATE_KEY,
} from "./constants";
import {
  decodeToken,
  decodeTokenList,
  encodeTokenList,
  normalizeFilterToken,
  normalizeServiceToken,
} from "./filters";
import type { PanelConfig, PanelState } from "./types";

function getRawQueryParam(name: string): string | null {
  const query = window.location.search.slice(1);
  if (!query) {
    return null;
  }
  const pairs = query.split("&");
  for (const pair of pairs) {
    if (!pair) {
      continue;
    }
    const [key, ...rest] = pair.split("=");
    if (key === name) {
      return rest.join("=");
    }
  }
  return null;
}

export function serializePanelConfig(panel: PanelState): string {
  const parts: string[] = [];
  const services = panel.filter ? [...panel.filter] : [];
  if (!panel.filter || services.length === 0) {
    parts.push("svc=all");
  } else {
    parts.push(`svc=${encodeTokenList(services)}`);
  }
  const includeTokens = panel.include.filter(Boolean);
  if (includeTokens.length) {
    parts.push(`inc=${encodeTokenList(includeTokens)}`);
  }
  const excludeTokens = panel.exclude.filter(Boolean);
  if (excludeTokens.length) {
    parts.push(`exc=${encodeTokenList(excludeTokens)}`);
  }
  if (!panel.autoScroll) {
    parts.push("follow=0");
  }
  return parts.join(GROUP_SEPARATOR);
}

export function serializePanelsConfig(panels: PanelState[]): string {
  if (!panels.length) {
    return "";
  }
  return panels.map((panel) => serializePanelConfig(panel)).join(PANEL_SEPARATOR);
}

export function parsePanelConfig(raw: string): PanelConfig {
  const config: PanelConfig = {
    services: null,
    include: [],
    exclude: [],
    follow: true,
  };
  if (!raw) {
    return config;
  }
  raw.split(GROUP_SEPARATOR).forEach((part) => {
    if (!part) {
      return;
    }
    const [key, ...rest] = part.split("=");
    const value = rest.join("=");
    if (key === "svc") {
      if (!value || value === "all") {
        config.services = null;
        return;
      }
      const services = decodeTokenList(value, normalizeServiceToken);
      config.services = services.length ? services : null;
      return;
    }
    if (key === "inc") {
      config.include = decodeTokenList(value, normalizeFilterToken);
      return;
    }
    if (key === "exc") {
      config.exclude = decodeTokenList(value, normalizeFilterToken);
      return;
    }
    if (key === "follow") {
      config.follow = value !== "0";
    }
  });
  return config;
}

export function parsePanelsConfig(raw: string | null): PanelConfig[] | null {
  if (!raw) {
    return null;
  }
  return raw
    .split(PANEL_SEPARATOR)
    .map((entry) => entry.trim())
    .filter(Boolean)
    .map((entry) => parsePanelConfig(entry));
}

export function parseActiveIndex(rawValue: string | null, panelCount?: number): number | null {
  if (!rawValue) {
    return null;
  }
  const decoded = decodeToken(rawValue);
  const parsed = Number.parseInt(decoded, 10);
  if (Number.isNaN(parsed)) {
    return null;
  }
  const index = parsed - 1;
  if (index < 0) {
    return null;
  }
  if (typeof panelCount === "number" && index >= panelCount) {
    return null;
  }
  return index;
}

export function readStateFromUrl(): {
  panels: PanelConfig[] | null;
  activeIndex: number | null;
} {
  const rawPanels = getRawQueryParam(URL_STATE_KEY);
  const rawActive = getRawQueryParam(URL_ACTIVE_KEY);
  const panels = rawPanels ? parsePanelsConfig(rawPanels) : null;
  const activeIndex = parseActiveIndex(rawActive, panels?.length);
  return { panels, activeIndex };
}

export function buildSearchString(panelsValue: string, activeIndex: number | null): string {
  const params = new URLSearchParams(window.location.search);
  params.delete(URL_STATE_KEY);
  params.delete(URL_ACTIVE_KEY);
  const parts: string[] = [];
  const base = params.toString();
  if (base) {
    parts.push(base);
  }
  if (panelsValue) {
    parts.push(`${URL_STATE_KEY}=${panelsValue}`);
  }
  if (activeIndex !== null) {
    parts.push(`${URL_ACTIVE_KEY}=${activeIndex + 1}`);
  }
  if (!parts.length) {
    return "";
  }
  return `?${parts.join("&")}`;
}
