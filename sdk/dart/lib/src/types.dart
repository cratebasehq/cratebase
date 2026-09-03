import 'package:http/http.dart' as http;

const Set<String> _recordKnownKeys = {
  'id',
  'created',
  'updated',
  'collectionId',
  'collectionName',
};

/// Every record carries these regardless of its collection's schema.
/// Extra schema fields are stashed in [data] and reachable via `operator[]`.
///
/// Records from an `auth`-typed collection additionally expose [email] and
/// [verified] (both nullable — absent for non-auth collections).
class RecordModel {
  final String id;
  final String created;
  final String updated;
  final String collectionId;
  final String collectionName;

  /// Every field not covered by the properties above (schema-defined data
  /// plus, for auth collections, `email`/`verified`).
  final Map<String, dynamic> data;

  RecordModel({
    required this.id,
    this.created = '',
    this.updated = '',
    this.collectionId = '',
    this.collectionName = '',
    Map<String, dynamic>? data,
  }) : data = data ?? {};

  factory RecordModel.fromJson(Map<String, dynamic> json) {
    final extra = <String, dynamic>{};
    json.forEach((key, value) {
      if (!_recordKnownKeys.contains(key)) extra[key] = value;
    });
    return RecordModel(
      id: json['id'] as String? ?? '',
      created: json['created'] as String? ?? '',
      updated: json['updated'] as String? ?? '',
      collectionId: json['collectionId'] as String? ?? '',
      collectionName: json['collectionName'] as String? ?? '',
      data: extra,
    );
  }

  Map<String, dynamic> toJson() => {
        'id': id,
        'created': created,
        'updated': updated,
        'collectionId': collectionId,
        'collectionName': collectionName,
        ...data,
      };

  /// Present when this record comes from an `auth`-typed collection.
  String? get email => data['email'] as String?;

  /// Present when this record comes from an `auth`-typed collection.
  bool? get verified => data['verified'] as bool?;

  /// Read any field — known or schema-defined — by name.
  dynamic operator [](String key) {
    switch (key) {
      case 'id':
        return id;
      case 'created':
        return created;
      case 'updated':
        return updated;
      case 'collectionId':
        return collectionId;
      case 'collectionName':
        return collectionName;
      default:
        return data[key];
    }
  }
}

class AdminModel {
  final String id;
  final String email;
  final String created;
  final String updated;
  final Map<String, dynamic> data;

  AdminModel({
    required this.id,
    required this.email,
    this.created = '',
    this.updated = '',
    Map<String, dynamic>? data,
  }) : data = data ?? {};

  factory AdminModel.fromJson(Map<String, dynamic> json) {
    const known = {'id', 'email', 'created', 'updated'};
    final extra = <String, dynamic>{};
    json.forEach((key, value) {
      if (!known.contains(key)) extra[key] = value;
    });
    return AdminModel(
      id: json['id'] as String? ?? '',
      email: json['email'] as String? ?? '',
      created: json['created'] as String? ?? '',
      updated: json['updated'] as String? ?? '',
      data: extra,
    );
  }

  Map<String, dynamic> toJson() => {
        'id': id,
        'email': email,
        'created': created,
        'updated': updated,
        ...data,
      };
}

class ListResult<T> {
  final int page;
  final int perPage;
  final int totalItems;
  final int totalPages;
  final List<T> items;

  const ListResult({
    required this.page,
    required this.perPage,
    required this.totalItems,
    required this.totalPages,
    required this.items,
  });

  factory ListResult.fromJson(
    Map<String, dynamic> json,
    T Function(Map<String, dynamic>) itemFromJson,
  ) {
    final rawItems = json['items'] as List<dynamic>? ?? const [];
    return ListResult<T>(
      page: json['page'] as int? ?? 1,
      perPage: json['perPage'] as int? ?? 0,
      totalItems: json['totalItems'] as int? ?? 0,
      totalPages: json['totalPages'] as int? ?? 0,
      items: rawItems.map((e) => itemFromJson(e as Map<String, dynamic>)).toList(),
    );
  }
}

enum FieldType {
  text,
  editor,
  number,
  bool,
  email,
  url,
  date,
  autodate,
  select,
  json,
  relation,
  file,
  password;

  static FieldType fromJson(String value) => FieldType.values.firstWhere(
        (type) => type.name == value,
        orElse: () => FieldType.text,
      );

  String toJson() => name;
}

class FieldSchema {
  final String id;
  final String name;
  final FieldType type;
  final bool required;
  final bool unique;
  final Map<String, dynamic>? options;

  const FieldSchema({
    this.id = '',
    required this.name,
    required this.type,
    this.required = false,
    this.unique = false,
    this.options,
  });

  factory FieldSchema.fromJson(Map<String, dynamic> json) => FieldSchema(
        id: json['id'] as String? ?? '',
        name: json['name'] as String? ?? '',
        type: FieldType.fromJson(json['type'] as String? ?? 'text'),
        required: json['required'] as bool? ?? false,
        unique: json['unique'] as bool? ?? false,
        options: json['options'] as Map<String, dynamic>?,
      );

  Map<String, dynamic> toJson() => {
        'id': id,
        'name': name,
        'type': type.toJson(),
        'required': required,
        'unique': unique,
        if (options != null) 'options': options,
      };
}

enum CollectionType {
  base,
  auth,
  view;

  static CollectionType fromJson(String value) => CollectionType.values.firstWhere(
        (type) => type.name == value,
        orElse: () => CollectionType.base,
      );

  String toJson() => name;
}

class AuthOptions {
  final int? minPasswordLength;

  /// Which schema field identifies an auth record for login. Defaults to
  /// `"email"`; set to `"username"` (or any other field name) to log in
  /// with something other than an email address.
  final String identityField;
  final bool requireEmailVerification;
  final int? tokenTtlSeconds;

  const AuthOptions({
    this.minPasswordLength,
    this.identityField = 'email',
    this.requireEmailVerification = false,
    this.tokenTtlSeconds,
  });

  factory AuthOptions.fromJson(Map<String, dynamic> json) => AuthOptions(
        minPasswordLength: json['minPasswordLength'] as int?,
        identityField: json['identityField'] as String? ?? 'email',
        requireEmailVerification: json['requireEmailVerification'] as bool? ?? false,
        tokenTtlSeconds: json['tokenTtlSeconds'] as int?,
      );

  Map<String, dynamic> toJson() => {
        if (minPasswordLength != null) 'minPasswordLength': minPasswordLength,
        'identityField': identityField,
        'requireEmailVerification': requireEmailVerification,
        if (tokenTtlSeconds != null) 'tokenTtlSeconds': tokenTtlSeconds,
      };
}

class CollectionModel {
  final String id;
  final String name;
  final CollectionType type;
  final List<FieldSchema> schema;
  final String? listRule;
  final String? viewRule;
  final String? createRule;
  final String? updateRule;
  final String? deleteRule;
  final AuthOptions? authOptions;
  final String created;
  final String updated;

  const CollectionModel({
    this.id = '',
    required this.name,
    required this.type,
    this.schema = const [],
    this.listRule,
    this.viewRule,
    this.createRule,
    this.updateRule,
    this.deleteRule,
    this.authOptions,
    this.created = '',
    this.updated = '',
  });

  factory CollectionModel.fromJson(Map<String, dynamic> json) => CollectionModel(
        id: json['id'] as String? ?? '',
        name: json['name'] as String? ?? '',
        type: CollectionType.fromJson(json['type'] as String? ?? 'base'),
        schema: (json['schema'] as List<dynamic>? ?? const [])
            .map((e) => FieldSchema.fromJson(e as Map<String, dynamic>))
            .toList(),
        listRule: json['listRule'] as String?,
        viewRule: json['viewRule'] as String?,
        createRule: json['createRule'] as String?,
        updateRule: json['updateRule'] as String?,
        deleteRule: json['deleteRule'] as String?,
        authOptions: json['authOptions'] != null
            ? AuthOptions.fromJson(json['authOptions'] as Map<String, dynamic>)
            : null,
        created: json['created'] as String? ?? '',
        updated: json['updated'] as String? ?? '',
      );

  Map<String, dynamic> toJson() => {
        'id': id,
        'name': name,
        'type': type.toJson(),
        'schema': schema.map((f) => f.toJson()).toList(),
        'listRule': listRule,
        'viewRule': viewRule,
        'createRule': createRule,
        'updateRule': updateRule,
        'deleteRule': deleteRule,
        if (authOptions != null) 'authOptions': authOptions!.toJson(),
        'created': created,
        'updated': updated,
      };

  /// Payload shape for create/update requests — the server assigns
  /// `id`/`created`/`updated`, so those are omitted here.
  Map<String, dynamic> toCreateJson() => {
        'name': name,
        'type': type.toJson(),
        'schema': schema.map((f) => f.toJson()).toList(),
        'listRule': listRule,
        'viewRule': viewRule,
        'createRule': createRule,
        'updateRule': updateRule,
        'deleteRule': deleteRule,
        if (authOptions != null) 'authOptions': authOptions!.toJson(),
      };
}

class AuthResponse {
  final String token;
  final RecordModel record;

  const AuthResponse({required this.token, required this.record});

  factory AuthResponse.fromJson(Map<String, dynamic> json) => AuthResponse(
        token: json['token'] as String? ?? '',
        record: RecordModel.fromJson(json['record'] as Map<String, dynamic>? ?? const {}),
      );
}

class AdminAuthResponse {
  final String token;
  final AdminModel admin;

  const AdminAuthResponse({required this.token, required this.admin});

  factory AdminAuthResponse.fromJson(Map<String, dynamic> json) => AdminAuthResponse(
        token: json['token'] as String? ?? '',
        admin: AdminModel.fromJson(json['admin'] as Map<String, dynamic>? ?? const {}),
      );
}

class OAuth2ProviderInfo {
  final String name;
  final String authUrl;

  const OAuth2ProviderInfo({required this.name, required this.authUrl});

  factory OAuth2ProviderInfo.fromJson(Map<String, dynamic> json) => OAuth2ProviderInfo(
        name: json['name'] as String? ?? '',
        authUrl: json['authUrl'] as String? ?? '',
      );
}

class OAuth2Info {
  final bool enabled;
  final List<OAuth2ProviderInfo> providers;

  const OAuth2Info({required this.enabled, required this.providers});

  factory OAuth2Info.fromJson(Map<String, dynamic> json) => OAuth2Info(
        enabled: json['enabled'] as bool? ?? false,
        providers: (json['providers'] as List<dynamic>? ?? const [])
            .map((e) => OAuth2ProviderInfo.fromJson(e as Map<String, dynamic>))
            .toList(),
      );
}

class AuthMethodsResponse {
  final bool password;
  final OAuth2Info oauth2;

  const AuthMethodsResponse({required this.password, required this.oauth2});

  factory AuthMethodsResponse.fromJson(Map<String, dynamic> json) => AuthMethodsResponse(
        password: json['password'] as bool? ?? false,
        oauth2: OAuth2Info.fromJson(json['oauth2'] as Map<String, dynamic>? ?? const {}),
      );
}

/// Multipart request payload for `create`/`update` calls against
/// collections with file fields — analogous to passing a browser `FormData`
/// to the JS SDK. Scalar fields go in [fields] (values are stringified;
/// pass a JSON string yourself for nested/array values), files in [files].
class CratebaseMultipart {
  final Map<String, dynamic> fields;
  final List<http.MultipartFile> files;

  const CratebaseMultipart({this.fields = const {}, this.files = const []});
}
