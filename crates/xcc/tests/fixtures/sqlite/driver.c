/* The program the sqlite3 fixture is compiled against: open an in-memory database, create a table,
 * insert rows with a multi-row VALUES list, and read them back through a prepared statement. Every
 * answer is printed, so a comparison with a `clang` build covers the parser, the b-tree, the record
 * format, the query planner and the result formatter at once.
 */
#include <stdio.h>

#include "sqlite3.h"

int main(void) {
  sqlite3 *db = 0;
  char *error = 0;
  sqlite3_stmt *statement = 0;

  if (sqlite3_open(":memory:", &db) != SQLITE_OK) {
    printf("open: %s\n", sqlite3_errmsg(db));
    return 1;
  }
  if (sqlite3_exec(db,
                   "CREATE TABLE t(a INTEGER, b TEXT);"
                   "INSERT INTO t VALUES(1,'one'),(2,'two'),(3,'three');",
                   0, 0, &error) != SQLITE_OK) {
    printf("exec: %s\n", error);
    return 2;
  }
  if (sqlite3_prepare_v2(db, "SELECT sum(a), group_concat(b), sqlite_version() FROM t", -1,
                         &statement, 0) != SQLITE_OK) {
    printf("prepare: %s\n", sqlite3_errmsg(db));
    return 3;
  }
  if (sqlite3_step(statement) != SQLITE_ROW) {
    printf("step: %s\n", sqlite3_errmsg(db));
    return 4;
  }
  printf("%d %s %s\n", sqlite3_column_int(statement, 0),
         (const char *)sqlite3_column_text(statement, 1),
         (const char *)sqlite3_column_text(statement, 2));
  sqlite3_finalize(statement);

  /* The aggregate over an empty table, which is a different path: no row was ever accumulated. */
  sqlite3_exec(db, "DELETE FROM t", 0, 0, 0);
  sqlite3_prepare_v2(db, "SELECT count(*), coalesce(sum(a), -1) FROM t", -1, &statement, 0);
  if (sqlite3_step(statement) == SQLITE_ROW) {
    printf("%d %d\n", sqlite3_column_int(statement, 0), sqlite3_column_int(statement, 1));
  }
  sqlite3_finalize(statement);
  sqlite3_close(db);
  return 0;
}
