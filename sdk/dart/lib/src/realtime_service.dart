import 'dart:async';
import 'dart:convert';

import 'package:http/http.dart' as http;

import 'client.dart';
import 'types.dart';

/// One realtime change notification for a record.
class RealtimeEvent {
  final String action; // "create" | "update" | "delete"
  final RecordModel record;

  const RealtimeEvent({required this.action, required this.record});

  factory RealtimeEvent.fromJson(Map<String, dynamic> json) => RealtimeEvent(
        action: json['action'] as String? ?? '',
        record: RecordModel.fromJson(json['record'] as Map<String, dynamic>? ?? const {}),
      );
}

typedef RealtimeCallback = void Function(RealtimeEvent event);

/// Unsubscribes a single [RealtimeService.subscribe] call.
typedef Unsubscribe = void Function();

/// Live record change subscriptions over Server-Sent Events. Subscribe to a
/// bare collection name for every change, or `"collection/recordId"` for
/// just one record.
///
/// ```dart
/// final unsubscribe = await cb.realtime.subscribe('posts', (e) {
///   print('${e.action} ${e.record.id}');
/// });
/// // later
/// unsubscribe();
/// ```
///
/// Dart has no built-in `EventSource` outside a browser context, so this
/// opens the stream itself via `http.Client().send()` and hand-parses the
/// `event:`/`data:` line protocol.
class RealtimeService {
  final CratebaseClient client;

  http.Client? _sseClient;
  StreamSubscription<String>? _subscription;
  String _clientId = '';
  Completer<void>? _connectCompleter;
  final Map<String, Set<RealtimeCallback>> _subscriptions = {};

  RealtimeService(this.client);

  Future<Unsubscribe> subscribe(String topic, RealtimeCallback callback) async {
    _subscriptions.putIfAbsent(topic, () => {}).add(callback);
    await _ensureConnected();
    await _syncSubscriptions();
    return () => unsubscribe(topic, callback);
  }

  Future<void> unsubscribe(String topic, [RealtimeCallback? callback]) async {
    final set = _subscriptions[topic];
    if (set == null) return;
    if (callback != null) {
      set.remove(callback);
    } else {
      set.clear();
    }
    if (set.isEmpty) _subscriptions.remove(topic);

    if (_subscriptions.isEmpty) {
      disconnect();
    } else {
      await _syncSubscriptions();
    }
  }

  void disconnect() {
    _subscription?.cancel();
    _subscription = null;
    _sseClient?.close();
    _sseClient = null;
    _clientId = '';
    _connectCompleter = null;
  }

  Future<void> _ensureConnected() {
    final existing = _connectCompleter;
    if (existing != null) return existing.future;

    final completer = Completer<void>();
    _connectCompleter = completer;

    final httpClient = http.Client();
    _sseClient = httpClient;

    final uri = Uri.parse('${client.baseUrl}/api/realtime');
    final request = http.Request('GET', uri);
    request.headers['accept'] = 'text/event-stream';

    unawaited(_runStream(httpClient, request, completer));

    return completer.future;
  }

  Future<void> _runStream(
    http.Client httpClient,
    http.Request request,
    Completer<void> completer,
  ) async {
    try {
      final response = await httpClient.send(request);
      if (response.statusCode != 200) {
        throw StateError('failed to connect to /api/realtime: HTTP ${response.statusCode}');
      }

      var eventName = 'message';
      final dataBuffer = StringBuffer();

      void dispatch() {
        final dataStr = dataBuffer.toString();
        dataBuffer.clear();
        final name = eventName;
        eventName = 'message';
        if (dataStr.isEmpty) return;

        dynamic parsed;
        try {
          parsed = jsonDecode(dataStr);
        } catch (_) {
          return;
        }
        if (parsed is! Map) return;

        if (name == 'PB_CONNECT') {
          final clientId = parsed['clientId'];
          if (clientId is String) {
            _clientId = clientId;
            if (!completer.isCompleted) completer.complete();
          }
          return;
        }

        if (parsed['action'] is! String || parsed['record'] is! Map) return;
        final event = RealtimeEvent.fromJson(parsed.cast<String, dynamic>());
        final topics = [
          event.record.collectionName,
          '${event.record.collectionName}/${event.record.id}',
        ];
        for (final topic in topics) {
          final callbacks = _subscriptions[topic];
          if (callbacks == null) continue;
          for (final cb in Set<RealtimeCallback>.from(callbacks)) {
            cb(event);
          }
        }
      }

      _subscription = response.stream
          .transform(utf8.decoder)
          .transform(const LineSplitter())
          .listen(
        (line) {
          if (line.isEmpty) {
            dispatch();
            return;
          }
          if (line.startsWith(':')) return; // SSE comment/keep-alive
          if (line.startsWith('event:')) {
            eventName = line.substring(6).trim();
          } else if (line.startsWith('data:')) {
            if (dataBuffer.length > 0) dataBuffer.write('\n');
            dataBuffer.write(line.substring(5).trim());
          }
          // `id:`/`retry:` lines are ignored — the server doesn't rely on them.
        },
        onError: (Object error) {
          if (!completer.isCompleted) completer.completeError(error);
          disconnect();
        },
        onDone: () {
          if (!completer.isCompleted) {
            completer.completeError(StateError('connection to /api/realtime closed before PB_CONNECT'));
          }
          disconnect();
        },
        cancelOnError: true,
      );
    } catch (error) {
      if (!completer.isCompleted) completer.completeError(error);
      disconnect();
    }
  }

  Future<void> _syncSubscriptions() async {
    if (_clientId.isEmpty) return;
    await client.send(
      '/api/realtime',
      method: 'POST',
      body: {'clientId': _clientId, 'subscriptions': _subscriptions.keys.toList()},
    );
  }
}
