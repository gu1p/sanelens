export interface ServiceInfo {
  name: string;
  endpoints?: string[];
  endpoint?: string | null;
  exposed?: boolean;
}

export interface LogEvent {
  seq: number;
  service: string;
  container_ts?: string | null;
  line: string;
}

export interface PanelState {
  id: string;
  title: string;
  filter: string[] | null;
  include: string[];
  exclude: string[];
  autoScroll: boolean;
  logs: LogEvent[];
  delay: number;
}

export interface PanelConfig {
  services: string[] | null;
  include: string[];
  exclude: string[];
  follow: boolean;
}
