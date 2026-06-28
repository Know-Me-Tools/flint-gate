import "dart:convert";

import "package:flutter/material.dart";
import "package:flint_gate/flint_gate.dart";

void main() {
  runApp(const FlintChatApp());
}

class FlintChatApp extends StatelessWidget {
  const FlintChatApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: "Flint Gate Chat",
      theme: ThemeData.dark(useMaterial3: true),
      home: const ChatPage(),
    );
  }
}

class ChatPage extends StatefulWidget {
  const ChatPage({super.key});

  @override
  State<ChatPage> createState() => _ChatPageState();
}

class _ChatPageState extends State<ChatPage> {
  final List<String> _messages = [];
  final TextEditingController _controller = TextEditingController();
  SseClient? _sse;

  static const String _baseUrl = String.fromEnvironment(
    "FLINT_GATE_URL",
    defaultValue: "http://127.0.0.1:4456",
  );
  static const String _token = String.fromEnvironment("FLINT_GATE_TOKEN");

  void _send(String text) {
    setState(() => _messages.add("You: $text"));
    _controller.clear();

    final url = Uri.parse("$_baseUrl/api/chat/stream").replace(
      queryParameters: {"message": text},
    );

    _sse?.close();
    _sse = SseClient(
      url: url.toString(),
      headers: _token.isNotEmpty
          ? {"Authorization": "Bearer $_token"}
          : const <String, String>{},
    );

    _sse!.connect().listen(
      _onEvent,
      onError: (Object err) => setState(() => _messages.add("Error: $err")),
      onDone: () => setState(() => _messages.add("[stream done]")),
    );
  }

  void _onEvent(SseEvent event) {
    final data = event.data.trim();
    if (data.isEmpty) return;

    setState(() {
      if (_messages.isNotEmpty && _messages.last.startsWith("Bot: ")) {
        _messages.last =
            "Bot: ${_messages.last.substring(5)}${utf8.decode(utf8.encode(data))}";
      } else {
        _messages.add("Bot: $data");
      }
    });
  }

  @override
  void dispose() {
    _sse?.close();
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Flint Gate Chat")),
      body: Column(
        children: [
          Expanded(
            child: ListView.builder(
              padding: const EdgeInsets.all(16),
              itemCount: _messages.length,
              itemBuilder: (_, i) => Text(_messages[i]),
            ),
          ),
          Padding(
            padding: const EdgeInsets.all(12),
            child: Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: _controller,
                    onSubmitted: _send,
                    decoration: const InputDecoration(
                      hintText: "Type a message...",
                      border: OutlineInputBorder(),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton(
                  icon: const Icon(Icons.send),
                  onPressed: () => _send(_controller.text),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
