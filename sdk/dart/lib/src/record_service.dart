import 'client.dart';
import 'types.dart';

/// CRUD + auth operations scoped to one collection. Auth-only methods
/// ([authWithPassword], [authRefresh], ...) only make sense for `auth`-typed
/// collections but are exposed here unconditionally since the client has no
/// way to know a collection's type ahead of a request.
class RecordService {
  final CratebaseClient client;
  final String collectionIdOrName;

  RecordService(this.client, this.collectionIdOrName);

  String get _basePath => '/api/collections/$collectionIdOrName/records';

  Future<ListResult<RecordModel>> getList({
    int page = 1,
    int perPage = 30,
    String? filter,
    String? sort,
  }) async {
    final json = await client.send(
      _basePath,
      query: {'page': page, 'perPage': perPage, 'filter': filter, 'sort': sort},
    );
    return ListResult.fromJson(json as Map<String, dynamic>, RecordModel.fromJson);
  }

  /// Fetches every page and concatenates the results. Convenient for small
  /// collections; prefer [getList] with pagination for large ones.
  Future<List<RecordModel>> getFullList({
    String? filter,
    String? sort,
    int batchSize = 200,
  }) async {
    final items = <RecordModel>[];
    var page = 1;
    for (;;) {
      final result = await getList(page: page, perPage: batchSize, filter: filter, sort: sort);
      items.addAll(result.items);
      if (page >= result.totalPages) break;
      page += 1;
    }
    return items;
  }

  Future<RecordModel> getOne(String id) async {
    final json = await client.send('$_basePath/$id');
    return RecordModel.fromJson(json as Map<String, dynamic>);
  }

  /// [data] is either a `Map<String, dynamic>` (JSON body) or a
  /// [CratebaseMultipart] for collections with file fields.
  Future<RecordModel> create(Object data) async {
    final json = await client.send(_basePath, method: 'POST', body: data);
    return RecordModel.fromJson(json as Map<String, dynamic>);
  }

  /// [data] is either a `Map<String, dynamic>` (JSON body) or a
  /// [CratebaseMultipart] for collections with file fields.
  Future<RecordModel> update(String id, Object data) async {
    final json = await client.send('$_basePath/$id', method: 'PATCH', body: data);
    return RecordModel.fromJson(json as Map<String, dynamic>);
  }

  Future<void> delete(String id) async {
    await client.send('$_basePath/$id', method: 'DELETE');
  }

  /// [identity] is whatever the collection's configured identity field is
  /// (email by default, but could be a username).
  Future<AuthResponse> authWithPassword(String identity, String password) async {
    final json = await client.send(
      '/api/collections/$collectionIdOrName/auth-with-password',
      method: 'POST',
      body: {'identity': identity, 'password': password},
    );
    final result = AuthResponse.fromJson(json as Map<String, dynamic>);
    client.authStore.save(result.token, result.record.toJson());
    return result;
  }

  Future<AuthResponse> authRefresh() async {
    final json = await client.send(
      '/api/collections/$collectionIdOrName/auth-refresh',
      method: 'POST',
    );
    final result = AuthResponse.fromJson(json as Map<String, dynamic>);
    client.authStore.save(result.token, result.record.toJson());
    return result;
  }

  /// Lists available auth methods. Pass your app's OAuth2 redirect
  /// URI/deep link and each provider's `authUrl` comes back ready to open
  /// directly.
  Future<AuthMethodsResponse> listAuthMethods({String? redirectUri}) async {
    final json = await client.send(
      '/api/collections/$collectionIdOrName/auth-methods',
      query: {'redirectUri': redirectUri},
    );
    return AuthMethodsResponse.fromJson(json as Map<String, dynamic>);
  }

  /// Completes an OAuth2 login: exchange the [code] your app received at
  /// [redirectUri] (the same one used to build the `authUrl` from
  /// [listAuthMethods]) for a session.
  Future<AuthResponse> authWithOAuth2(String provider, String code, String redirectUri) async {
    final json = await client.send(
      '/api/collections/$collectionIdOrName/auth-with-oauth2',
      method: 'POST',
      body: {'provider': provider, 'code': code, 'redirectUri': redirectUri},
    );
    final result = AuthResponse.fromJson(json as Map<String, dynamic>);
    client.authStore.save(result.token, result.record.toJson());
    return result;
  }

  /// Sends a verification email if [email] matches an account — always
  /// resolves regardless, so it can't be used to enumerate accounts.
  Future<void> requestVerification(String email) async {
    await client.send(
      '/api/collections/$collectionIdOrName/request-verification',
      method: 'POST',
      body: {'email': email},
    );
  }

  /// Confirms a verification token from the emailed link.
  Future<void> confirmVerification(String token) async {
    await client.send(
      '/api/collections/$collectionIdOrName/confirm-verification',
      method: 'POST',
      body: {'token': token},
    );
  }

  /// Sends a password reset email if [email] matches an account — always
  /// resolves regardless, so it can't be used to enumerate accounts.
  Future<void> requestPasswordReset(String email) async {
    await client.send(
      '/api/collections/$collectionIdOrName/request-password-reset',
      method: 'POST',
      body: {'email': email},
    );
  }

  /// Confirms a password reset token and sets a new password.
  Future<void> confirmPasswordReset(String token, String password, String passwordConfirm) async {
    await client.send(
      '/api/collections/$collectionIdOrName/confirm-password-reset',
      method: 'POST',
      body: {'token': token, 'password': password, 'passwordConfirm': passwordConfirm},
    );
  }

  /// Requires the current session's record to be authenticated. Sends a
  /// confirmation link to [newEmail] — the identity only changes once that
  /// link is confirmed, proving ownership of the new address.
  Future<void> requestEmailChange(String newEmail) async {
    await client.send(
      '/api/collections/$collectionIdOrName/request-email-change',
      method: 'POST',
      body: {'newEmail': newEmail},
    );
  }

  /// Confirms an email-change token from the emailed link.
  Future<void> confirmEmailChange(String token) async {
    await client.send(
      '/api/collections/$collectionIdOrName/confirm-email-change',
      method: 'POST',
      body: {'token': token},
    );
  }
}
