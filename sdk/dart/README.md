# cratebase

Official Dart client SDK for [Cratebase](../../README.md). Pure Dart (no
Flutter dependency) — works from CLI apps, servers, and Flutter apps alike.

```yaml
dependencies:
  cratebase: ^0.1.0
```

```dart
import 'package:cratebase/cratebase.dart';

final cb = CratebaseClient('http://127.0.0.1:8090');

// auth
await cb.collection('users').authWithPassword('alice@example.com', 'secret123');

// records
final page = await cb.collection('posts').getList(
  page: 1,
  perPage: 20,
  filter: 'published = true',
  sort: '-created',
);
final post = await cb.collection('posts').create({'title': 'Hello', 'published': true});

// files: pass a CratebaseMultipart for collections with file fields
final doc = await cb.collection('docs').create(CratebaseMultipart(
  fields: {'title': 'Report'},
  files: [await http.MultipartFile.fromPath('attachment', './report.pdf')],
));
final url = cb.getFileUrl(doc, doc['attachment'] as String);

// realtime
final unsubscribe = await cb.realtime.subscribe('posts', (e) {
  print('${e.action} ${e.record.id}');
});
// later
unsubscribe();
```

## Auth persistence

`AuthStore` holds the current token/model in memory for the process
lifetime. This package has no Flutter dependency, so it ships no default
disk persistence — wire your own by implementing `AuthStorePersistence`
(e.g. backed by `SharedPreferences` or `flutter_secure_storage` in a
Flutter app) and passing it in:

```dart
class PrefsAuthPersistence implements AuthStorePersistence {
  @override
  Future<void> save(String token, Map<String, dynamic>? model) async {
    // write to SharedPreferences / flutter_secure_storage
  }

  @override
  Future<void> clear() async {
    // remove from storage
  }

  @override
  Future<AuthStoreRecord?> load() async {
    // read from storage, or return null if nothing saved
    return null;
  }
}

final authStore = await AuthStore.restore(PrefsAuthPersistence());
final cb = CratebaseClient('http://127.0.0.1:8090', authStore: authStore);
```

See the [API reference](../../openapi.yaml) for the full request/response
shapes this client wraps.
