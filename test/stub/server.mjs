import { createServer } from "node:http";

const NDJSON_LINES = [
  JSON.stringify({
    type: "TOOL_CALL_START",
    toolCallId: "tc-001",
    toolCallName: "integ_test_tool",
  }),
  JSON.stringify({
    type: "TOOL_CALL_ARGS",
    toolCallId: "tc-001",
    delta: '{"x":1}',
  }),
  JSON.stringify({ type: "TOOL_CALL_END", toolCallId: "tc-001" }),
];

createServer((req, res) => {
  if (req.url === "/health") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ status: "ok" }));
    return;
  }

  res.writeHead(200, {
    "Content-Type": "application/x-ndjson",
    "Transfer-Encoding": "chunked",
    "Cache-Control": "no-cache",
  });

  let i = 0;
  const interval = setInterval(() => {
    if (i >= NDJSON_LINES.length) {
      clearInterval(interval);
      res.end();
      return;
    }
    res.write(NDJSON_LINES[i++] + "\n");
  }, 50);

  req.on("close", () => clearInterval(interval));
}).listen(9999, () => {
  console.log("mock-upstream listening on :9999");
});
