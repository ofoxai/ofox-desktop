# ZIP compatibility fixture

`zip-2.4.2-stored.zip` was generated with the application's `zip` 2.4.2
dependency before the security migration. It contains one stored member,
`safe.txt`, whose contents are `test archive payload`.

Keep this file frozen. Reading a fixture from the previous writer protects
compatibility independently of round trips through the current writer. The
inconsistent-header tests construct and alter their own archives in memory;
this fixture contains no malicious paths or user data.

`zip-2.4.2-deflated.zip` and `zip-2.4.2-zip64.zip` were generated with the
same previous library using a real file writer and deflate compression. The
latter explicitly enables its ZIP64 end record. Both contain `safe.txt` and
`notes/说明.txt`, exercising compressed data, Unicode names and ZIP64 metadata
independently of the current writer.
