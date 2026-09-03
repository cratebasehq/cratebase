import 'client.dart';
import 'types.dart';

/// Manage collection schemas. Requires a superuser token in
/// `client.authStore`.
class SchemaService {
  final CratebaseClient client;

  SchemaService(this.client);

  Future<List<CollectionModel>> getList() async {
    final json = await client.send('/api/collections');
    return (json as List<dynamic>)
        .map((e) => CollectionModel.fromJson(e as Map<String, dynamic>))
        .toList();
  }

  Future<CollectionModel> getOne(String idOrName) async {
    final json = await client.send('/api/collections/$idOrName');
    return CollectionModel.fromJson(json as Map<String, dynamic>);
  }

  Future<CollectionModel> create(CollectionModel data) async {
    final json = await client.send(
      '/api/collections',
      method: 'POST',
      body: data.toCreateJson(),
    );
    return CollectionModel.fromJson(json as Map<String, dynamic>);
  }

  Future<CollectionModel> update(String idOrName, CollectionModel data) async {
    final json = await client.send(
      '/api/collections/$idOrName',
      method: 'PATCH',
      body: data.toCreateJson(),
    );
    return CollectionModel.fromJson(json as Map<String, dynamic>);
  }

  Future<void> delete(String idOrName) async {
    await client.send('/api/collections/$idOrName', method: 'DELETE');
  }
}
