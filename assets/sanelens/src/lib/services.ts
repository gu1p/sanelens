import type { ServiceInfo } from "./types";

export function getEndpoints(service: ServiceInfo): string[] {
  if (Array.isArray(service.endpoints) && service.endpoints.length) {
    return service.endpoints;
  }
  if (service.endpoint) {
    return [service.endpoint];
  }
  return [];
}

export function endpointLabel(endpoint: string): string {
  try {
    const url = new URL(endpoint);
    return url.host;
  } catch (error) {
    return endpoint.replace("http://", "");
  }
}
