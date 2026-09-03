import 'dart:convert';

/// Called whenever [AuthStore.save] or [AuthStore.clear] changes the store.
typedef AuthChangeListener = void Function(String token, Map<String, dynamic>? model);

/// One saved token + model pair, as returned by [AuthStorePersistence.load].
class AuthStoreRecord {
  final String token;
  final Map<String, dynamic>? model;

  const AuthStoreRecord(this.token, this.model);
}

/// Pluggable persistence for [AuthStore]. This package has no Flutter
/// dependency, so it ships no default implementation beyond in-memory —
/// a Flutter app wires `SharedPreferences` or `flutter_secure_storage` by
/// implementing this interface and passing it to [AuthStore.new] or
/// [AuthStore.restore].
abstract class AuthStorePersistence {
  Future<void> save(String token, Map<String, dynamic>? model);
  Future<void> clear();
  Future<AuthStoreRecord?> load();
}

/// Holds the current auth token + record/admin for one [CratebaseClient].
/// Kept in memory for the process lifetime unless a [AuthStorePersistence]
/// is supplied, in which case every [save]/[clear] is mirrored to it
/// fire-and-forget (persistence failures never block a request).
class AuthStore {
  final AuthStorePersistence? persistence;
  final List<AuthChangeListener> _listeners = [];
  String _token;
  Map<String, dynamic>? _model;

  AuthStore({this.persistence, String token = '', Map<String, dynamic>? model})
      : _token = token,
        _model = model;

  /// Builds an [AuthStore] pre-populated from [persistence] — use this
  /// instead of the default constructor when you want the store hydrated
  /// with a previously saved session before the first request goes out.
  static Future<AuthStore> restore(AuthStorePersistence persistence) async {
    final record = await persistence.load();
    return AuthStore(persistence: persistence, token: record?.token ?? '', model: record?.model);
  }

  String get token => _token;

  Map<String, dynamic>? get model => _model;

  /// `true` when a token is set and, if it looks like a JWT, isn't expired.
  /// Non-JWT tokens (or tokens whose payload has no `exp`) are treated as
  /// always valid — the server is the source of truth either way.
  bool get isValid {
    if (_token.isEmpty) return false;
    final exp = _decodeJwtExp(_token);
    if (exp == null) return true;
    return exp * 1000 > DateTime.now().millisecondsSinceEpoch;
  }

  void save(String token, Map<String, dynamic>? model) {
    _token = token;
    _model = model;
    unawaited(persistence?.save(token, model));
    _emit();
  }

  void clear() {
    _token = '';
    _model = null;
    unawaited(persistence?.clear());
    _emit();
  }

  /// Registers [listener]; returns a function that unregisters it.
  void Function() onChange(AuthChangeListener listener) {
    _listeners.add(listener);
    return () => _listeners.remove(listener);
  }

  void _emit() {
    for (final listener in List<AuthChangeListener>.from(_listeners)) {
      listener(_token, _model);
    }
  }
}

void unawaited(Future<void>? future) {
  // Deliberately not awaited: persistence is best-effort and must never
  // block or fail a save()/clear() call. Errors are swallowed rather than
  // left as unhandled Future rejections.
  future?.catchError((_) {});
}

int? _decodeJwtExp(String token) {
  final parts = token.split('.');
  if (parts.length != 3) return null;
  try {
    var payload = parts[1];
    payload += '=' * ((4 - payload.length % 4) % 4);
    final json = utf8.decode(base64Url.decode(payload));
    final decoded = jsonDecode(json);
    if (decoded is Map && decoded['exp'] is int) return decoded['exp'] as int;
    return null;
  } catch (_) {
    return null;
  }
}
