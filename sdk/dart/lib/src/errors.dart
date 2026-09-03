/// A single field's validation failure: a stable machine-readable [code]
/// (e.g. `"value_too_short"`) alongside a human-readable [message].
class FieldError {
  final String code;
  final String message;

  const FieldError({required this.code, required this.message});

  factory FieldError.fromJson(Map<String, dynamic> json) => FieldError(
        code: json['code'] as String? ?? '',
        message: json['message'] as String? ?? '',
      );

  Map<String, dynamic> toJson() => {'code': code, 'message': message};

  @override
  String toString() => '$code: $message';
}

/// Thrown for any non-2xx response from the Cratebase API. [data] carries
/// per-field validation errors when [status] is `400`.
class ClientResponseError implements Exception {
  final String url;
  final int status;
  final String message;
  final Map<String, FieldError> data;

  const ClientResponseError({
    required this.url,
    required this.status,
    required this.message,
    this.data = const {},
  });

  factory ClientResponseError.fromResponse(
    String url,
    int status,
    Map<String, dynamic>? body,
  ) {
    final rawData = body?['data'];
    final data = <String, FieldError>{};
    if (rawData is Map) {
      rawData.forEach((key, value) {
        if (value is Map<String, dynamic>) {
          data[key as String] = FieldError.fromJson(value);
        }
      });
    }
    final message =
        body?['message'] as String? ?? 'request to $url failed with status $status';
    return ClientResponseError(url: url, status: status, message: message, data: data);
  }

  @override
  String toString() => 'ClientResponseError($status): $message';
}
