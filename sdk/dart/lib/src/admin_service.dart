import 'client.dart';
import 'types.dart';

/// Superuser (admin panel) session management.
class AdminService {
  final CratebaseClient client;

  AdminService(this.client);

  Future<AdminAuthResponse> authWithPassword(String email, String password) async {
    final json = await client.send(
      '/api/admins/auth-with-password',
      method: 'POST',
      body: {'email': email, 'password': password},
    );
    final result = AdminAuthResponse.fromJson(json as Map<String, dynamic>);
    client.authStore.save(result.token, result.admin.toJson());
    return result;
  }

  Future<AdminAuthResponse> authRefresh() async {
    final json = await client.send('/api/admins/auth-refresh', method: 'POST');
    final result = AdminAuthResponse.fromJson(json as Map<String, dynamic>);
    client.authStore.save(result.token, result.admin.toJson());
    return result;
  }
}
