extern crate "cargo-registry" as cargo_registry;
extern crate migrate;
extern crate postgres;
extern crate r2d2;

use std::os;
use std::collections::HashSet;
use migrate::Migration;
use postgres::{PostgresTransaction, PostgresResult};

use cargo_registry::krate::Crate;

fn main() {
    let db_config = r2d2::Config {
        pool_size: 1,
        helper_tasks: 1,
        test_on_check_out: false,
    };
    let database = cargo_registry::db::pool(env("DATABASE_URL").as_slice(),
                                            db_config);
    let conn = database.get().unwrap();
    let migrations = migrations();

    if os::args().as_slice().get(1).map(|s| s.as_slice()) == Some("rollback") {
        rollback(conn.transaction().unwrap(), migrations).unwrap();
    } else {
        apply(conn.transaction().unwrap(), migrations).unwrap();
    }

    fn env(s: &str) -> String {
        match os::getenv(s) {
            Some(s) => s,
            None => fail!("must have `{}` defined", s),
        }
    }
}

fn apply(tx: PostgresTransaction,
         migrations: Vec<Migration>) -> PostgresResult<()> {
    let mut mgr = try!(migrate::Manager::new(tx));
    for m in migrations.into_iter() {
        try!(mgr.apply(m));
    }
    mgr.set_commit();
    mgr.finish()
}

fn rollback(tx: PostgresTransaction,
            migrations: Vec<Migration>) -> PostgresResult<()> {
    let mut mgr = try!(migrate::Manager::new(tx));
    for m in migrations.into_iter().rev() {
        if mgr.contains(m.version()) {
            try!(mgr.rollback(m));
            break
        }
    }
    mgr.set_commit();
    mgr.finish()
}

fn migrations() -> Vec<Migration> {
    // Generate a new id via `date +"%Y%m%d%H%M%S"`
    let migrations = vec![
        Migration::add_table(20140924113530, "users", "
            id              SERIAL PRIMARY KEY,
            email           VARCHAR NOT NULL UNIQUE,
            gh_access_token VARCHAR NOT NULL,
            api_token       VARCHAR NOT NULL
        "),
        Migration::add_table(20140924114003, "packages", "
            id              SERIAL PRIMARY KEY,
            name            VARCHAR NOT NULL UNIQUE,
            user_id         INTEGER NOT NULL
        "),
        Migration::add_table(20140924114059, "versions", "
            id              SERIAL PRIMARY KEY,
            package_id      INTEGER NOT NULL,
            num             VARCHAR NOT NULL
        "),
        Migration::run(20140924115329,
                       format!("ALTER TABLE versions ADD CONSTRAINT \
                                unique_num UNIQUE (package_id, num)"),
                       format!("ALTER TABLE versions DROP CONSTRAINT \
                                unique_num")),
        Migration::add_table(20140924120803, "version_dependencies", "
            version_id      INTEGER NOT NULL,
            depends_on_id   INTEGER NOT NULL
        "),
        Migration::add_column(20140925132248, "packages", "updated_at",
                              "TIMESTAMP NOT NULL DEFAULT now()"),
        Migration::add_column(20140925132249, "packages", "created_at",
                              "TIMESTAMP NOT NULL DEFAULT now()"),
        Migration::new(20140925132250, proc(tx) {
            try!(tx.execute("UPDATE packages SET updated_at = now() \
                             WHERE updated_at IS NULL", []));
            try!(tx.execute("UPDATE packages SET created_at = now() \
                             WHERE created_at IS NULL", []));
            Ok(())
        }, proc(_) Ok(())),
        Migration::add_column(20140925132251, "versions", "updated_at",
                              "TIMESTAMP NOT NULL DEFAULT now()"),
        Migration::add_column(20140925132252, "versions", "created_at",
                              "TIMESTAMP NOT NULL DEFAULT now()"),
        Migration::new(20140925132253, proc(tx) {
            try!(tx.execute("UPDATE versions SET updated_at = now() \
                             WHERE updated_at IS NULL", []));
            try!(tx.execute("UPDATE versions SET created_at = now() \
                             WHERE created_at IS NULL", []));
            Ok(())
        }, proc(_) Ok(())),
        Migration::new(20140925132254, proc(tx) {
            try!(tx.execute("ALTER TABLE versions ALTER COLUMN updated_at \
                             DROP DEFAULT", []));
            try!(tx.execute("ALTER TABLE versions ALTER COLUMN created_at \
                             DROP DEFAULT", []));
            try!(tx.execute("ALTER TABLE packages ALTER COLUMN updated_at \
                             DROP DEFAULT", []));
            try!(tx.execute("ALTER TABLE packages ALTER COLUMN created_at \
                             DROP DEFAULT", []));
            Ok(())
        }, proc(_) Ok(())),
        Migration::add_table(20140925153704, "metadata", "
            total_downloads        BIGINT NOT NULL
        "),
        Migration::new(20140925153705, proc(tx) {
            try!(tx.execute("INSERT INTO metadata (total_downloads) \
                             VALUES ($1)", &[&0i64]));
            Ok(())
        }, proc(tx) {
            try!(tx.execute("DELETE FROM metadata", [])); Ok(())
        }),
        Migration::add_column(20140925161623, "packages", "downloads",
                              "INTEGER NOT NULL DEFAULT 0"),
        Migration::add_column(20140925161624, "versions", "downloads",
                              "INTEGER NOT NULL DEFAULT 0"),
        Migration::new(20140925161625, proc(tx) {
            try!(tx.execute("ALTER TABLE versions ALTER COLUMN downloads \
                             DROP DEFAULT", []));
            try!(tx.execute("ALTER TABLE packages ALTER COLUMN downloads \
                             DROP DEFAULT", []));
            Ok(())
        }, proc(_) Ok(())),
        Migration::add_column(20140926130044, "packages", "max_version",
                              "VARCHAR"),
        Migration::new(20140926130045, proc(tx) {
            let stmt = try!(tx.prepare("SELECT * FROM packages"));
            for row in try!(stmt.query(&[])) {
                let pkg = Crate::from_row(&row);
                let versions = pkg.versions(tx).unwrap();
                let v = versions.iter().max_by(|v| &v.num).unwrap();
                let max = v.num.to_string();
                try!(tx.execute("UPDATE packages SET max_version = $1 \
                                 WHERE id = $2",
                                &[&max, &pkg.id]));
            }
            Ok(())
        }, proc(_) Ok(())),
        Migration::new(20140926130046, proc(tx) {
            try!(tx.execute("ALTER TABLE versions ALTER COLUMN downloads \
                             SET NOT NULL", []));
            Ok(())
        }, proc(tx) {
            try!(tx.execute("ALTER TABLE versions ALTER COLUMN downloads \
                             DROP NOT NULL", []));
            Ok(())
        }),
        Migration::new(20140926174020, proc(tx) {
            try!(tx.execute("ALTER TABLE packages RENAME TO crates", []));
            try!(tx.execute("ALTER TABLE versions RENAME COLUMN package_id \
                             TO crate_id", []));
            Ok(())
        }, proc(tx) {
            try!(tx.execute("ALTER TABLE crates RENAME TO packages", []));
            try!(tx.execute("ALTER TABLE versions RENAME COLUMN crate_id \
                             TO package_id", []));
            Ok(())
        }),
        Migration::run(20140929103749,
                       "CREATE INDEX index_crate_updated_at ON crates (updated_at)",
                       "DROP INDEX index_crate_updated_at"),
        Migration::run(20140929103750,
                       "CREATE INDEX index_crate_created_at ON crates (created_at)",
                       "DROP INDEX index_crate_created_at"),
        Migration::run(20140929103751,
                       "CREATE INDEX index_crate_downloads ON crates (downloads)",
                       "DROP INDEX index_crate_downloads"),
        Migration::run(20140929103752,
                       "CREATE INDEX index_version_crate_id ON versions (crate_id)",
                       "DROP INDEX index_version_crate_id"),
        Migration::run(20140929103753,
                       "CREATE INDEX index_version_num ON versions (num)",
                       "DROP INDEX index_version_num"),
        Migration::run(20140929103754,
                       "CREATE INDEX index_version_dependencies_version_id \
                        ON version_dependencies (version_id)",
                       "DROP INDEX index_version_dependencies_version_id"),
        Migration::run(20140929103755,
                       "CREATE INDEX index_version_dependencies_depends_on_id \
                        ON version_dependencies (depends_on_id)",
                       "DROP INDEX index_version_dependencies_depends_on_id"),
        Migration::add_table(20140929103756, "crate_downloads", "
            crate_id        INTEGER NOT NULL,
            downloads       INTEGER NOT NULL,
            date            TIMESTAMP NOT NULL,
            processed       BOOLEAN NOT NULL
        "),
        Migration::run(20140929103757,
                       "CREATE INDEX index_crate_downloads_crate_id \
                        ON crate_downloads (crate_id)",
                       "DROP INDEX index_crate_downloads_crate_id"),
        Migration::run(20140929103758,
                       "CREATE INDEX index_crate_downloads_date \
                        ON crate_downloads (date(date))",
                       "DROP INDEX index_crate_downloads_date"),
        Migration::add_table(20140929103759, "version_downloads", "
            version_id      INTEGER NOT NULL,
            downloads       INTEGER NOT NULL,
            date            TIMESTAMP NOT NULL,
            processed       BOOLEAN NOT NULL
        "),
        Migration::run(20140929103760,
                       "CREATE INDEX index_version_downloads_version_id \
                        ON version_downloads (version_id)",
                       "DROP INDEX index_version_downloads_version_id"),
        Migration::run(20140929103761,
                       "CREATE INDEX index_version_downloads_date \
                        ON version_downloads (date(date))",
                       "DROP INDEX index_version_downloads_date"),
        Migration::run(20140929103762,
                       "CREATE INDEX index_crate_downloads_processed \
                        ON crate_downloads (processed)
                        WHERE processed = FALSE",
                       "DROP INDEX index_crate_downloads_processed"),
        Migration::run(20140929103763,
                       "CREATE INDEX index_version_downloads_processed \
                        ON version_downloads (processed)
                        WHERE processed = FALSE",
                       "DROP INDEX index_version_downloads_processed"),
    ];

    let mut seen = HashSet::new();
    for m in migrations.iter() {
        if !seen.insert(m.version()) {
            fail!("duplicate id: {}", m.version());
        }
    }
    return migrations;
}
