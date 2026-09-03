import 'dart:convert';

import 'package:http/http.dart' as http;

import 'admin_service.dart';
import 'auth_store.dart';
import 'errors.dart';
import 'realtime_service.dart';
import 'record_service.dart';
import 'schema_service.dart';
import 'types.dart';

/// Cratebase API client. One instance per backend URL; safe to share as a
/// singleton across your app — records/collections/auth all read
/// [authStore] lazily on every request, so logging in updates every
/// in-flight caller.
///
/// ```dart
/// final cb = CratebaseClient('https://api.example.com');
/// await cb.collection('users').authWithPassword('a@b.com', 'secret');
/// final posts = await cb.collection('posts').getList(perPage: 20, filter: 'published = true');
/// ```
class CratebaseClient {
  final String baseUrl;
  final AuthStore authStore;
  final http.Client _httpClient;

  late final AdminService admins;
  late final SchemaService collections;
  late final RealtimeService realtime;

  CratebaseClient(String baseUrl, {AuthStore? authStore, http.Client? httpClient})
      : baseUrl = _stripTrailingSlashes(baseUrl),
        authStore = authStore ?? AuthStore(),
        _httpClient = httpClient ?? http.Client() {
    admins = AdminService(this);
    collections = SchemaService(this);
    realtime = RealtimeService(this);
  }

  static String _stripTrailingSlashes(String url) {
    var end = url.length;
    while (end > 0 && url[end - 1] == '/') {
      end -= 1;
    }
    return url.substring(0, end);
  }

  /// Get a [RecordService] bound to one collection (by id or name).
  RecordService collection(String idOrName) => RecordService(this, idOrName);

  /// The download URL for a file field's stored filename.
  String getFileUrl(RecordModel record, String filename) {
    final collection = record.collectionName.isNotEmpty ? record.collectionName : record.collectionId;
    return '$baseUrl/api/files/$collection/${record.id}/$filename';
  }

  /// Low-level request helper every service is built on. Throws
  /// [ClientResponseError] for non-2xx responses.
  ///
  /// [body] is either a `Map<String, dynamic>` (sent as JSON) or a
  /// [CratebaseMultipart] (sent as `multipart/form-data`, for collections
  /// with file fields). Returns `null` for `204 No Content` responses.
  Future<dynamic> send(
    String path, {
    String method = 'GET',
    Object? body,
    Map<String, String>? headers,
    Map<String, dynamic>? query,
  }) async {
    final queryParams = <String, String>{};
    query?.forEach((key, value) {
      if (value != null) queryParams[key] = value.toString();
    });
    final uri = Uri.parse('$baseUrl$path').replace(
      queryParameters: queryParams.isEmpty ? null : queryParams,
    );

    final requestHeaders = <String, String>{...?headers};
    if (authStore.token.isNotEmpty) {
      requestHeaders['authorization'] = 'Bearer ${authStore.token}';
    }

    http.StreamedResponse streamed;
    if (body is CratebaseMultipart) {
      final request = http.MultipartRequest(method, uri)..headers.addAll(requestHeaders);
      body.fields.forEach((key, value) {
        if (value != null) request.fields[key] = value is String ? value : jsonEncode(value);
      });
      request.files.addAll(body.files);
      streamed = await _httpClient.send(request);
    } else {
      final request = http.Request(method, uri)..headers.addAll(requestHeaders);
      if (body != null) {
        request.headers['content-type'] = 'application/json';
        request.body = jsonEncode(body);
      }
      streamed = await _httpClient.send(request);
    }

    final response = await http.Response.fromStream(streamed);
    if (response.statusCode == 204) return null;

    final text = response.body;
    final dynamic data = text.isEmpty ? null : jsonDecode(text);

    if (response.statusCode < 200 || response.statusCode >= 300) {
      throw ClientResponseError.fromResponse(
        uri.toString(),
        response.statusCode,
        data is Map<String, dynamic> ? data : null,
      );
    }
    return data;
  }

  /// Releases the underlying HTTP client and any open realtime connection.
  void close() {
    realtime.disconnect();
    _httpClient.close();
  }
}
