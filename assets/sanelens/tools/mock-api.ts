import type { Connect, Plugin, ViteDevServer } from "vite";
import type { LogEvent, ServiceInfo } from "../src/lib/types";

const MOCK_HISTORY_LIMIT = 400;
const MOCK_SEED_COUNT = 160;
const MOCK_INTERVAL_MS = 900;

export function mockApiPlugin(): Plugin {
  const services: ServiceInfo[] = [
    { name: "edge-gateway", endpoint: "https://api.sanelens.local" },
    { name: "auth-service", endpoint: "https://auth.sanelens.local" },
    {
      name: "catalog-api",
      endpoints: ["https://catalog.sanelens.local", "https://cdn.sanelens.local"],
    },
    { name: "orders-service", endpoint: "https://orders.sanelens.local" },
    { name: "payments-service", endpoint: "https://payments.sanelens.local" },
    { name: "search-service", endpoint: "https://search.sanelens.local" },
    { name: "notifications-worker" },
    { name: "inventory-sync" },
    { name: "postgres" },
    { name: "redis" },
  ];

  const users = [
    "maya",
    "tariq",
    "riley",
    "morgan",
    "jules",
    "devin",
    "sam",
    "alex",
    "casey",
    "noah",
  ];
  const skus = ["LN-1002", "LN-2004", "FR-3201", "AC-4410", "KT-9912", "CL-8802", "PR-2231"];
  const regions = ["us-east-1", "us-west-2", "eu-central-1", "ap-southeast-1"];
  const searchTerms = [
    "clear lens",
    "night drive",
    "retro frame",
    "blue light",
    "ultralight",
    "polarized",
    "sport fit",
  ];
  const paths = [
    "/api/catalog",
    "/api/orders",
    "/api/search",
    "/api/checkout",
    "/api/auth/session",
    "/api/cart",
  ];
  const methods = ["GET", "POST", "PUT"];
  const collections = ["spring", "studio", "sport", "commuter"];

  type SseClient = { write: (chunk: string) => void };
  type RequestLike = {
    url?: string;
    on: (event: "close", listener: () => void) => void;
  };
  type ResponseLike = {
    statusCode: number;
    setHeader: (name: string, value: string) => void;
    end: (data?: string) => void;
    write: (data: string) => void;
  };

  const history: LogEvent[] = [];
  const clients = new Set<SseClient>();
  let seq = 1;
  let ticker: ReturnType<typeof setInterval> | null = null;

  const pad = (value: number, size = 2) => String(value).padStart(size, "0");
  const formatTimestamp = (date: Date) =>
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}.${pad(
      date.getMilliseconds(),
      3
    )}`;

  const randomInt = (min: number, max: number) =>
    Math.floor(Math.random() * (max - min + 1)) + min;
  const pick = <T>(list: T[]) => list[randomInt(0, list.length - 1)];
  const randomId = (prefix: string) => `${prefix}_${Math.random().toString(36).slice(2, 8)}`;
  const randomIp = () => `10.${randomInt(0, 255)}.${randomInt(0, 255)}.${randomInt(0, 255)}`;
  const randomMoney = () => (randomInt(1200, 42000) / 100).toFixed(2);

  const logGenerators: Record<string, Array<() => string>> = {
    "edge-gateway": [
      () =>
        `${pick(methods)} ${pick(paths)} ${pick([200, 201, 204, 304, 404, 429])} ${randomInt(
          18,
          240
        )}ms`,
      () =>
        `proxy upstream=${pick(["catalog-api", "orders-service", "search-service"])} ${randomInt(
          12,
          120
        )}ms`,
      () =>
        `warn upstream reset service=${pick([
          "catalog-api",
          "orders-service",
          "payments-service",
        ])} retry=${randomInt(1, 3)}`,
      () => `rate_limit ip=${randomIp()} bucket=global wait=${randomInt(20, 140)}ms`,
    ],
    "auth-service": [
      () => `login success user=${pick(users)} method=password`,
      () => `token refresh user=${pick(users)} ttl=${randomInt(30, 240)}m`,
      () => `warn invalid password user=${pick(users)} ip=${randomIp()}`,
      () => `error oauth timeout provider=okta retry=${randomInt(1, 3)}`,
    ],
    "catalog-api": [
      () => `GET /items/${pick(skus)} 200 ${randomInt(40, 160)}ms`,
      () => `cache hit key=item:${pick(skus)}`,
      () => `cache miss key=collection:${pick(collections)}`,
      () => `warn image fallback sku=${pick(skus)}`,
    ],
    "orders-service": [
      () => `created order id=${randomId("ord")} total=$${randomMoney()}`,
      () => `reserved inventory sku=${pick(skus)} qty=${randomInt(1, 4)}`,
      () => `warn fulfillment delay order=${randomId("ord")} region=${pick(regions)}`,
      () => `error payment capture failed order=${randomId("ord")} retrying`,
    ],
    "payments-service": [
      () => `charge authorized id=${randomId("pay")} amount=$${randomMoney()} method=card`,
      () => `capture settled id=${randomId("pay")} batch=${randomInt(100, 999)}`,
      () => `warn gateway latency ${randomInt(900, 1800)}ms`,
      () => `error gateway timeout provider=stripe retry=${randomInt(1, 3)}`,
    ],
    "search-service": [
      () =>
        `query "${pick(searchTerms)}" hits=${randomInt(2, 120)} ${randomInt(20, 160)}ms`,
      () => `indexed ${randomInt(50, 400)} docs in ${randomInt(200, 1200)}ms`,
      () =>
        `warn slow shard shard=${randomInt(1, 6)} latency=${randomInt(400, 1200)}ms`,
    ],
    "notifications-worker": [
      () => `sent email template=order-confirmation user=${pick(users)}`,
      () => `sent sms template=otp user=${pick(users)}`,
      () => `queue depth=${randomInt(80, 240)} lag=${randomInt(200, 900)}ms`,
      () => `retry push notification id=${randomId("msg")}`,
      () => `error provider response 502 retrying`,
    ],
    "inventory-sync": [
      () => `synced sku=${pick(skus)} delta=${randomInt(-4, 10)}`,
      () => `snapshot imported items=${randomInt(1200, 5200)}`,
      () => `warn supplier feed lag=${randomInt(5, 45)}s`,
      () => `error feed parse failure line=${randomInt(200, 1200)}`,
    ],
    postgres: [
      () =>
        `checkpoint complete: ${randomInt(120, 320)} buffers, ${randomInt(1, 4)}.${randomInt(
          0,
          9
        )}s`,
      () => `autovacuum: VACUUM public.orders`,
      () => `slow query ${randomInt(450, 1400)}ms`,
      () => `warning: replication lag ${randomInt(120, 620)}ms`,
    ],
    redis: [
      () => `connected clients=${randomInt(80, 160)} used_memory=${randomInt(400, 980)}mb`,
      () => `evicted keys=${randomInt(10, 240)} policy=allkeys-lru`,
      () => `persistence rdb saved in ${randomInt(60, 180)}ms`,
      () => `warn memory pressure ratio=${(randomInt(65, 92) / 100).toFixed(2)}`,
    ],
  };

  const createEntry = (service: string, line: string, time = new Date()): LogEvent => ({
    seq: seq++,
    service,
    container_ts: formatTimestamp(time),
    line,
  });

  const addToHistory = (entry: LogEvent) => {
    history.push(entry);
    if (history.length > MOCK_HISTORY_LIMIT) {
      history.shift();
    }
  };

  const pickLine = (service: string) => {
    const generators = logGenerators[service];
    if (!generators || generators.length === 0) {
      return `log event service=${service}`;
    }
    return pick(generators)();
  };

  const broadcast = (entry: LogEvent) => {
    addToHistory(entry);
    const payload = `data: ${JSON.stringify(entry)}\n\n`;
    clients.forEach((res) => res.write(payload));
  };

  const seedHistory = () => {
    const now = Date.now();
    for (let i = MOCK_SEED_COUNT; i > 0; i -= 1) {
      const service = services[i % services.length].name;
      const line = pickLine(service);
      const time = new Date(now - i * 900 - randomInt(0, 400));
      addToHistory(createEntry(service, line, time));
    }
  };

  seedHistory();

  return {
    name: "mock-api",
    configureServer(server: ViteDevServer) {
      if (!ticker) {
        ticker = setInterval(() => {
          const service = pick(services).name;
          broadcast(createEntry(service, pickLine(service)));
        }, MOCK_INTERVAL_MS);
      }

      const handleMock: Connect.NextHandleFunction = (req, res, next) => {
        const request = req as RequestLike;
        const response = res as ResponseLike;
        const path = request.url?.split("?")[0] ?? "";
        if (path === "/api/services") {
          response.statusCode = 200;
          response.setHeader("Content-Type", "application/json");
          response.setHeader("Cache-Control", "no-cache");
          response.end(JSON.stringify({ services }));
          return;
        }

        if (path === "/events") {
          response.statusCode = 200;
          response.setHeader("Content-Type", "text/event-stream");
          response.setHeader("Cache-Control", "no-cache");
          response.setHeader("Connection", "keep-alive");
          response.write(`event: history\ndata: ${JSON.stringify(history)}\n\n`);
          clients.add(response);
          request.on("close", () => {
            clients.delete(response);
          });
          return;
        }

        next();
      };

      server.middlewares.use(handleMock);
    },
  };
}
