import { createFileRoute } from "@tanstack/react-router";
import { createServerFn } from "@tanstack/react-start";
import { env } from "cloudflare:workers";

const getWhoami = createServerFn({ method: "GET" }).handler(async () => {
  const { call } = await import("../linear.server");
  return call("whoami", {}, env.LINEAR_API_KEY);
});

export const Route = createFileRoute("/")({
  loader: () => getWhoami(),
  component: Home,
});

function Home() {
  const result = Route.useLoaderData();
  return (
    <main>
      <h1>linear-core via wasm, from TanStack Start on Workers</h1>
      <pre id="result">{JSON.stringify(result, null, 2)}</pre>
    </main>
  );
}
