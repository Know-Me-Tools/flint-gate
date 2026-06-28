import {
  FlintGateClient,
  streamSSE,
  type StreamEvent,
} from "@know-me/flint-gate";

const GATE_URL = process.env.FLINT_GATE_URL ?? "http://127.0.0.1:4456";
const AUTH_TOKEN = process.env.FLINT_GATE_TOKEN;

const client = new FlintGateClient({
  baseUrl: GATE_URL,
  auth: AUTH_TOKEN
    ? { type: "bearer", token: AUTH_TOKEN }
    : { type: "anonymous" },
});

async function main() {
  const res = await client.requestStream("/api/chat/completions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      model: "claude-sonnet-4-6",
      messages: [{ role: "user", content: "Explain Flint Gate in one sentence." }],
      stream: true,
    }),
  });

  let fullText = "";
  for await (const ev of streamSSE(res)) {
    printEvent(ev);
    if (ev.type === "text-delta") {
      fullText += ev.text;
    }
    if (ev.type === "done" || ev.type === "error") {
      break;
    }
  }

  console.log("\n--- complete message ---");
  console.log(fullText);
}

function printEvent(ev: StreamEvent) {
  switch (ev.type) {
    case "text-delta":
      process.stdout.write(ev.text);
      break;
    case "tool-call":
      console.log(`\n[tool-call ${ev.name} id=${ev.id}]`);
      break;
    case "done":
      console.log("\n[stream done]");
      if (ev.usage) {
        console.log("usage:", ev.usage);
      }
      break;
    case "error":
      console.error("\n[stream error]", ev.message);
      break;
    default:
      console.log("\n[unknown event]", ev);
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
