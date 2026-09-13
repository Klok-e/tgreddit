use crate::{config::*, reddit::*, types::*};
use anyhow::{Context, Result};
use rusqlite::{Connection, Row, named_params};
use rusqlite::{
    OptionalExtension,
    types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef},
};
use rusqlite_migration::{M, Migrations};
use std::path::Path;
use std::str::FromStr;
use std::string::ToString;
use std::{convert::TryFrom, sync::Mutex};
use teloxide::types::{FileId, FileUniqueId, InlineKeyboardMarkup, MessageId};

use crate::types::{MediaKind, TelegramMediaFile};

const MIGRATIONS: &[&str] = &[
    "
    create table post(
        post_id     text not null,
        chat_id     integer not null,
        subreddit   text not null,
        seen_at     text not null,
        primary key (post_id, chat_id)
    ) strict;
    ",
    "
    create table subscription(
        chat_id     integer not null,
        subreddit   text not null,
        created_at  text not null,
        post_limit  integer,
        time        text,
        filter      text,
        primary key (subreddit, chat_id)
    ) strict;
    ",
    "
    create table chat(
        chat_id     integer primary key,
        repost_channel_id integer
    ) strict;
    ",
    "
    insert or ignore into chat (chat_id)
    select chat_id from subscription;
    ",
    "
    create table subscription_new(
        chat_id     integer not null,
        subreddit   text not null,
        created_at  text not null,
        post_limit  integer,
        time        text,
        filter      text,
        primary key (subreddit, chat_id),
        foreign key (chat_id) references chat(chat_id)
    );
    ",
    "
    insert into subscription_new
    select * from subscription;
    ",
    "
    drop table subscription;
    ",
    "
    alter table subscription_new
    rename to subscription;
    ",
    "
    create table post_new(
        post_id     text not null,
        chat_id     integer not null,
        subreddit   text not null,
        seen_at     text, -- make seen_at nullable
        post_title  text not null, -- new field
        primary key (post_id, chat_id)
    ) strict;
    ",
    "
    insert into post_new (post_id, chat_id, subreddit, seen_at, post_title)
    select post_id, chat_id, subreddit, seen_at, 'Unknown' as post_title from post;
    ",
    "
    drop table post;
    ",
    "
    alter table post_new
    rename to post;
    ",
    "
    create table telegram_file(
        id                  integer primary key autoincrement,
        post_id             text not null,
        chat_id             integer not null,
        telegram_file_id    text not null,
        foreign key (post_id, chat_id) references post(post_id, chat_id)
    ) strict;
    ",
    "
    create table telegram_file_new(
        post_id                    text not null,
        chat_id                    integer not null,
        telegram_file_id           text not null,
        telegram_file_unique_id    text not null,
        foreign key (post_id, chat_id) references post(post_id, chat_id),
        primary key (post_id, chat_id, telegram_file_unique_id)
    ) strict;
    ",
    "
    drop table telegram_file;
    ",
    "
    alter table telegram_file_new
    rename to telegram_file;
    ",
    "
    CREATE TABLE telegram_file_new(
        id                  INTEGER PRIMARY KEY AUTOINCREMENT,
        post_id             TEXT NOT NULL,
        chat_id             INTEGER NOT NULL,
        telegram_file_id    TEXT NOT NULL,
        telegram_file_unique_id    TEXT NOT NULL,
        FOREIGN KEY (post_id, chat_id) REFERENCES post(post_id, chat_id),
        UNIQUE (post_id, chat_id, telegram_file_unique_id)
    ) STRICT;
    ",
    "
    INSERT INTO telegram_file_new (post_id, chat_id, telegram_file_id, telegram_file_unique_id)
    SELECT post_id, chat_id, telegram_file_id, telegram_file_unique_id FROM telegram_file;
    ",
    "
    DROP TABLE telegram_file;
    ",
    "
    ALTER TABLE telegram_file_new RENAME TO telegram_file;
    ",
    "
    create table review_post(
        chat_id                       integer not null,
        post_id                       text not null,
        source_url                    text not null,
        caption_text                  text not null,
        caption_entities_json         text not null,
        content_kind_json             text not null,
        review_message_id             integer not null,
        control_message_id            integer not null,
        metadata_text                 text not null,
        metadata_entities_json        text not null,
        pending_publish_variant_json  text,
        previous_keyboard_json        text,
        publishing                    integer not null default 0 check (publishing in (0, 1)),
        published_at                  text,
        primary key (chat_id, post_id),
        foreign key (post_id, chat_id) references post(post_id, chat_id),
        unique (chat_id, control_message_id)
    ) strict;
    ",
    "
    create table review_post_new(
        chat_id                       integer not null,
        post_id                       text not null,
        source_url                    text not null,
        caption_text                  text not null,
        caption_entities_json         text not null,
        content_kind_json             text not null,
        review_message_id             integer not null,
        control_message_id            integer not null,
        metadata_text                 text not null,
        metadata_entities_json        text not null,
        pending_publish_variant_json  text,
        previous_keyboard_json        text,
        publishing                    integer not null default 0 check (publishing in (0, 1)),
        published_at                  text,
        primary key (chat_id, post_id),
        unique (chat_id, control_message_id)
    ) strict;
    ",
    "
    insert into review_post_new
    select * from review_post;
    ",
    "
    drop table review_post;
    ",
    "
    alter table review_post_new rename to review_post;
    ",
    "
    alter table telegram_file
    add column media_kind text not null default 'photo' check (media_kind in ('photo', 'video'));
    ",
    "
    alter table review_post
    add column caption_above_media integer not null default 0 check (caption_above_media in (0, 1));
    ",
];

#[derive(Debug)]
pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn open(config: &Config) -> Result<Self> {
        let conn = Self::get_conn(&config.db_path).context("error connecting to database")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Ok(Database {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    fn get_conn(_db_path: &Path) -> Result<Connection, rusqlite::Error> {
        Connection::open_in_memory()
    }

    #[cfg(not(test))]
    fn get_conn(db_path: &Path) -> Result<Connection, rusqlite::Error> {
        std::fs::create_dir_all(db_path.parent().expect("Db path doesn't contain a file"))
            .expect("Couldn't create directory for db file");
        Connection::open(db_path)
    }

    pub fn migrate(&mut self) -> Result<(), rusqlite_migration::Error> {
        let migrations = MIGRATIONS.iter().map(|e| M::up(e)).collect();
        Migrations::new(migrations).to_latest(&mut self.conn.lock().expect("No poison"))
    }

    pub fn record_post<T: Recordable>(
        &self,
        chat_id: i64,
        post: &T,
        seen_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        // First, attempt to insert a new row with INSERT OR IGNORE
        let conn = self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            insert or ignore into post (post_id, chat_id, subreddit, seen_at, post_title)
            values (:post_id, :chat_id, :subreddit, :seen_at, :post_title)
            ",
        )?;
        stmt.execute(named_params! {
            ":post_id": post.id(),
            ":chat_id": chat_id,
            ":subreddit": &post.subreddit(),
            ":seen_at": seen_at,
            ":post_title": &post.title(),
        })?;

        // Then, update the seen_at field for the row with the given post_id and chat_id, only if seen_at is null
        let mut stmt = conn.prepare(
            "
            update post
            set seen_at = :seen_at
            where post_id = :post_id and chat_id = :chat_id and seen_at is null
            ",
        )?;
        stmt.execute(named_params! {
            ":seen_at": seen_at,
            ":post_id": post.id(),
            ":chat_id": chat_id,
        })
        .context("could not update seen_at")
        .map(|_| ())
    }

    pub fn record_post_seen_with_current_time<T: Recordable>(
        &self,
        chat_id: i64,
        post: &T,
    ) -> Result<()> {
        let current_time = Some(chrono::Utc::now());
        self.record_post(chat_id, post, current_time)
    }

    pub fn get_post_title(&self, chat_id: i64, post_id: &str) -> Result<String> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select post_title
            from post
            where post_id = :post_id and chat_id = :chat_id
            ",
        )?;

        let post_title = stmt
            .query_row(
                named_params! {
                    ":post_id": post_id,
                    ":chat_id": chat_id,
                },
                |row| row.get("post_title"),
            )
            .context("could not retrieve post title")?;

        Ok(post_title)
    }

    pub fn is_post_seen<T: Recordable>(&self, chat_id: i64, post: &T) -> Result<bool> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select exists(
                select 1 
                  from post
                 where post_id = :post_id and chat_id = :chat_id and seen_at is not null
            );
            ",
        )?;

        stmt.query_row(
            named_params! {
                ":post_id": post.id(),
                ":chat_id": chat_id
            },
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    }

    pub fn existing_posts_for_subreddit(&self, chat_id: i64, subreddit: &str) -> Result<bool> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select exists(
                select 1
                  from post
                 where chat_id = :chat_id and subreddit = :subreddit
            );
            ",
        )?;

        stmt.query_row(
            named_params! {
                ":chat_id": chat_id,
                ":subreddit": subreddit,
            },
            |row| row.get(0),
        )
        .map_err(anyhow::Error::from)
    }

    pub fn subscribe(&self, chat_id: i64, args: &SubscriptionArgs) -> Result<()> {
        self.ensure_chat_exists(chat_id)?;

        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            insert or replace into subscription (chat_id, subreddit, post_limit, time, filter, created_at)
            values (:chat_id, :subreddit, :limit, :time, :filter, :created_at)
            ",
        )?;
        stmt.execute(named_params! {
            ":chat_id": chat_id,
            ":subreddit": args.subreddit,
            ":limit": args.limit,
            ":time": args.time,
            ":filter": args.filter,
            ":created_at": chrono::Utc::now()
        })
        .context("could not add subscription")?;
        Ok(())
    }

    pub fn unsubscribe(&self, chat_id: i64, subreddit: &str) -> Result<String> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            delete from subscription
            where chat_id = :chat_id and subreddit LIKE :subreddit
            returning subreddit
            ",
        )?;
        let deleted_subreddit: String = stmt
            .query_row(
                named_params! {
                    ":chat_id": chat_id,
                    ":subreddit": subreddit,
                },
                |row| row.get("subreddit"),
            )
            .context("could not delete subscription")?;

        Ok(deleted_subreddit)
    }

    pub fn get_subscriptions_for_chat(&self, chat_id: i64) -> Result<Vec<Subscription>> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select chat_id, subreddit, post_limit, time, filter, created_at
            from subscription
            where chat_id = ?
            ",
        )?;

        let subs = stmt
            .query_map([chat_id], |row| Subscription::try_from(row))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;

        Ok(subs)
    }

    pub fn get_all_subscriptions(&self) -> Result<Vec<Subscription>> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select chat_id, subreddit, post_limit, time, filter, created_at
            from subscription
            ",
        )?;

        let subs = stmt
            .query_map([], |row| Subscription::try_from(row))?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;

        Ok(subs)
    }

    pub fn ensure_chat_exists(&self, chat_id: i64) -> Result<()> {
        let conn = &self.conn.lock().expect("No poison");
        let chat_exists: bool = conn.query_row(
            "
            select exists(
                select 1
                from chat
                where chat_id = :chat_id
            );
            ",
            named_params! {
                ":chat_id": chat_id,
            },
            |row| row.get(0),
        )?;

        if !chat_exists {
            let mut stmt = conn.prepare(
                "
                insert into chat (chat_id)
                values (:chat_id);
                ",
            )?;

            stmt.execute(named_params! {
                ":chat_id": chat_id,
            })
            .context("could not create chat")?;
        }

        Ok(())
    }

    pub fn set_repost_channel(&self, chat_id: i64, repost_channel_id: i64) -> Result<()> {
        self.ensure_chat_exists(chat_id)?;
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            update chat
            set repost_channel_id = :repost_channel_id
            where chat_id = :chat_id;
            ",
        )?;

        stmt.execute(named_params! {
            ":chat_id": chat_id,
            ":repost_channel_id": repost_channel_id,
        })
        .context("could not set repost channel")?;

        Ok(())
    }

    pub fn get_repost_channel(&self, chat_id: i64) -> Result<Option<i64>> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select repost_channel_id
            from chat
            where chat_id = :chat_id;
            ",
        )?;

        let repost_channel_id: Option<i64> = stmt
            .query_row(
                named_params! {
                    ":chat_id": chat_id,
                },
                |row| row.get("repost_channel_id"),
            )
            .optional()
            .context("could not get repost channel")?;

        Ok(repost_channel_id)
    }

    pub fn add_telegram_file(
        &self,
        post_id: &str,
        chat_id: i64,
        telegram_file_id: &FileId,
        telegram_unique_file_id: &FileUniqueId,
        media_kind: MediaKind,
    ) -> Result<()> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            insert or ignore into telegram_file (post_id, chat_id, telegram_file_id, telegram_file_unique_id, media_kind)
            values (:post_id, :chat_id, :telegram_file_id, :telegram_file_unique_id, :media_kind)
            ",
        )?;
        stmt.execute(named_params! {
            ":post_id": post_id,
            ":chat_id": chat_id,
            ":telegram_file_id": telegram_file_id.0,
            ":telegram_file_unique_id": telegram_unique_file_id.0,
            ":media_kind": media_kind.as_db_value(),
        })
        .context("could not add telegram file")
        .map(|_| ())
    }

    pub fn get_telegram_files_for_post(
        &self,
        post_id: &str,
        chat_id: i64,
    ) -> Result<Vec<TelegramMediaFile>> {
        let conn = &self.conn.lock().expect("No poison");
        let mut stmt = conn.prepare(
            "
            select telegram_file_id, media_kind
            from telegram_file
            where post_id = :post_id and chat_id = :chat_id
            order by telegram_file.id
            ",
        )?;

        let rows = stmt
            .query_map(
                named_params! {
                    ":post_id": post_id,
                    ":chat_id": chat_id,
                },
                |row| {
                    let file_id: String = row.get("telegram_file_id")?;
                    let media_kind: String = row.get("media_kind")?;
                    let kind = MediaKind::from_db_value(&media_kind).ok_or_else(|| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            format!("unsupported stored media kind {media_kind:?}").into(),
                        )
                    })?;
                    Ok(TelegramMediaFile {
                        file_id: file_id.into(),
                        kind,
                    })
                },
            )
            .context("could not retrieve telegram files")?;

        let telegram_files: Result<Vec<TelegramMediaFile>, _> = rows.collect();
        Ok(telegram_files?)
    }

    pub fn upsert_review_post(&self, review: &ReviewPost) -> Result<()> {
        let caption_entities_json = serde_json::to_string(&review.caption.entities)
            .context("could not serialize review caption entities")?;
        let content_kind_json = serde_json::to_string(&review.content_kind)
            .context("could not serialize review content kind")?;
        let metadata_entities_json = serde_json::to_string(&review.metadata.entities)
            .context("could not serialize review metadata entities")?;
        let pending_publish_variant_json = review
            .pending_publish_variant
            .map(|variant| serde_json::to_string(&variant))
            .transpose()
            .context("could not serialize pending publish variant")?;
        let previous_keyboard_json = review
            .previous_keyboard
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("could not serialize previous review keyboard")?;

        let conn = self.conn.lock().expect("No poison");
        conn.execute(
            "
            insert into review_post (
                chat_id,
                post_id,
                source_url,
                caption_text,
                caption_entities_json,
                content_kind_json,
                caption_above_media,
                review_message_id,
                control_message_id,
                metadata_text,
                metadata_entities_json,
                pending_publish_variant_json,
                previous_keyboard_json,
                published_at
            ) values (
                :chat_id,
                :post_id,
                :source_url,
                :caption_text,
                :caption_entities_json,
                :content_kind_json,
                :caption_above_media,
                :review_message_id,
                :control_message_id,
                :metadata_text,
                :metadata_entities_json,
                :pending_publish_variant_json,
                :previous_keyboard_json,
                :published_at
            )
            on conflict (chat_id, post_id) do update set
                source_url = excluded.source_url,
                caption_text = excluded.caption_text,
                caption_entities_json = excluded.caption_entities_json,
                content_kind_json = excluded.content_kind_json,
                caption_above_media = excluded.caption_above_media,
                review_message_id = excluded.review_message_id,
                control_message_id = excluded.control_message_id,
                metadata_text = excluded.metadata_text,
                metadata_entities_json = excluded.metadata_entities_json,
                pending_publish_variant_json = excluded.pending_publish_variant_json,
                previous_keyboard_json = excluded.previous_keyboard_json,
                publishing = 0,
                published_at = excluded.published_at
            ",
            named_params! {
                ":chat_id": review.chat_id,
                ":post_id": review.post_id,
                ":source_url": review.source_url,
                ":caption_text": review.caption.text,
                ":caption_entities_json": caption_entities_json,
                ":content_kind_json": content_kind_json,
                ":caption_above_media": review.caption_placement.is_above_media(),
                ":review_message_id": review.review_message_id.0,
                ":control_message_id": review.control_message_id.0,
                ":metadata_text": review.metadata.text,
                ":metadata_entities_json": metadata_entities_json,
                ":pending_publish_variant_json": pending_publish_variant_json,
                ":previous_keyboard_json": previous_keyboard_json,
                ":published_at": review.published_at,
            },
        )
        .context("could not save review post")?;
        Ok(())
    }

    pub fn get_review_post(&self, chat_id: i64, post_id: &str) -> Result<Option<ReviewPost>> {
        self.query_review_post(
            "where chat_id = :chat_id and post_id = :post_id",
            named_params! {
                ":chat_id": chat_id,
                ":post_id": post_id,
            },
        )
    }

    pub fn get_review_post_by_control_message(
        &self,
        chat_id: i64,
        control_message_id: MessageId,
    ) -> Result<Option<ReviewPost>> {
        self.query_review_post(
            "where chat_id = :chat_id and control_message_id = :control_message_id",
            named_params! {
                ":chat_id": chat_id,
                ":control_message_id": control_message_id.0,
            },
        )
    }

    pub fn active_media_review_posts(&self) -> Result<Vec<ReviewPost>> {
        let conn = self.conn.lock().expect("No poison");
        let mut statement = conn
            .prepare(
                "
                select chat_id,
                       post_id,
                       source_url,
                       caption_text,
                       caption_entities_json,
                       content_kind_json,
                       caption_above_media,
                       review_message_id,
                       control_message_id,
                       metadata_text,
                       metadata_entities_json,
                       pending_publish_variant_json,
                       previous_keyboard_json,
                       published_at
                from review_post
                where content_kind_json in ('\"m\"', '\"g\"')
                  and pending_publish_variant_json is null
                  and publishing = 0
                  and published_at is null
                ",
            )
            .context("could not query active media review posts")?;
        let rows = statement
            .query_map([], |row| {
                Ok(StoredReviewPost {
                    chat_id: row.get("chat_id")?,
                    post_id: row.get("post_id")?,
                    source_url: row.get("source_url")?,
                    caption_text: row.get("caption_text")?,
                    caption_entities_json: row.get("caption_entities_json")?,
                    content_kind_json: row.get("content_kind_json")?,
                    caption_above_media: row.get("caption_above_media")?,
                    review_message_id: row.get("review_message_id")?,
                    control_message_id: row.get("control_message_id")?,
                    metadata_text: row.get("metadata_text")?,
                    metadata_entities_json: row.get("metadata_entities_json")?,
                    pending_publish_variant_json: row.get("pending_publish_variant_json")?,
                    previous_keyboard_json: row.get("previous_keyboard_json")?,
                    published_at: row.get("published_at")?,
                })
            })
            .context("could not retrieve active media review posts")?;
        rows.map(|stored| ReviewPost::try_from(stored?)).collect()
    }

    pub fn update_review_caption(
        &self,
        chat_id: i64,
        post_id: &str,
        caption: &RichText,
    ) -> Result<()> {
        let entities_json = serde_json::to_string(&caption.entities)
            .context("could not serialize review caption entities")?;
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set caption_text = :caption_text,
                    caption_entities_json = :caption_entities_json
                where chat_id = :chat_id and post_id = :post_id
                ",
                named_params! {
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                    ":caption_text": caption.text,
                    ":caption_entities_json": entities_json,
                },
            )
            .context("could not update review caption")?;
        anyhow::ensure!(changed == 1, "review post does not exist");
        Ok(())
    }

    pub fn update_caption_placement(
        &self,
        chat_id: i64,
        post_id: &str,
        placement: CaptionPlacement,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set caption_above_media = :caption_above_media
                where chat_id = :chat_id
                  and post_id = :post_id
                  and pending_publish_variant_json is null
                  and publishing = 0
                  and published_at is null
                ",
                named_params! {
                    ":caption_above_media": placement.is_above_media(),
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                },
            )
            .context("could not update caption placement")?;
        anyhow::ensure!(
            changed == 1,
            "unpublished review post is not ready for caption placement"
        );
        Ok(())
    }

    pub fn begin_review_publish(
        &self,
        chat_id: i64,
        post_id: &str,
        variant: PublishVariant,
        previous_keyboard: Option<&InlineKeyboardMarkup>,
    ) -> Result<()> {
        let variant_json =
            serde_json::to_string(&variant).context("could not serialize publish variant")?;
        let previous_keyboard_json = previous_keyboard
            .map(serde_json::to_string)
            .transpose()
            .context("could not serialize previous review keyboard")?;
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set pending_publish_variant_json = :pending_publish_variant_json,
                    previous_keyboard_json = :previous_keyboard_json
                where chat_id = :chat_id
                  and post_id = :post_id
                  and publishing = 0
                  and published_at is null
                ",
                named_params! {
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                    ":pending_publish_variant_json": variant_json,
                    ":previous_keyboard_json": previous_keyboard_json,
                },
            )
            .context("could not select review publish variant")?;
        anyhow::ensure!(changed == 1, "unpublished review post does not exist");
        Ok(())
    }

    pub fn clear_review_publish(&self, chat_id: i64, post_id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set pending_publish_variant_json = null,
                    previous_keyboard_json = null,
                    publishing = 0
                where chat_id = :chat_id and post_id = :post_id
                ",
                named_params! {
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                },
            )
            .context("could not clear review publish state")?;
        anyhow::ensure!(changed == 1, "review post does not exist");
        Ok(())
    }

    pub fn claim_review_publish(&self, chat_id: i64, post_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set publishing = 1
                where chat_id = :chat_id
                  and post_id = :post_id
                  and pending_publish_variant_json is not null
                  and publishing = 0
                  and published_at is null
                ",
                named_params! {
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                },
            )
            .context("could not claim review publication")?;
        Ok(changed == 1)
    }

    pub fn mark_review_published(&self, chat_id: i64, post_id: &str) -> Result<()> {
        let conn = self.conn.lock().expect("No poison");
        let changed = conn
            .execute(
                "
                update review_post
                set pending_publish_variant_json = null,
                    previous_keyboard_json = null,
                    publishing = 0,
                    published_at = :published_at
                where chat_id = :chat_id
                  and post_id = :post_id
                  and publishing = 1
                  and published_at is null
                ",
                named_params! {
                    ":chat_id": chat_id,
                    ":post_id": post_id,
                    ":published_at": chrono::Utc::now(),
                },
            )
            .context("could not mark review post published")?;
        anyhow::ensure!(changed == 1, "unpublished review post does not exist");
        Ok(())
    }

    fn query_review_post(
        &self,
        filter: &str,
        params: impl rusqlite::Params,
    ) -> Result<Option<ReviewPost>> {
        let conn = self.conn.lock().expect("No poison");
        let sql = format!(
            "
            select chat_id,
                   post_id,
                   source_url,
                   caption_text,
                   caption_entities_json,
                   content_kind_json,
                   caption_above_media,
                   review_message_id,
                   control_message_id,
                   metadata_text,
                   metadata_entities_json,
                   pending_publish_variant_json,
                   previous_keyboard_json,
                   published_at
            from review_post
            {filter}
            "
        );
        let stored = conn
            .query_row(&sql, params, |row| {
                Ok(StoredReviewPost {
                    chat_id: row.get("chat_id")?,
                    post_id: row.get("post_id")?,
                    source_url: row.get("source_url")?,
                    caption_text: row.get("caption_text")?,
                    caption_entities_json: row.get("caption_entities_json")?,
                    content_kind_json: row.get("content_kind_json")?,
                    caption_above_media: row.get("caption_above_media")?,
                    review_message_id: row.get("review_message_id")?,
                    control_message_id: row.get("control_message_id")?,
                    metadata_text: row.get("metadata_text")?,
                    metadata_entities_json: row.get("metadata_entities_json")?,
                    pending_publish_variant_json: row.get("pending_publish_variant_json")?,
                    previous_keyboard_json: row.get("previous_keyboard_json")?,
                    published_at: row.get("published_at")?,
                })
            })
            .optional()
            .context("could not retrieve review post")?;
        stored.map(ReviewPost::try_from).transpose()
    }
}

struct StoredReviewPost {
    chat_id: i64,
    post_id: String,
    source_url: String,
    caption_text: String,
    caption_entities_json: String,
    content_kind_json: String,
    caption_above_media: bool,
    review_message_id: i32,
    control_message_id: i32,
    metadata_text: String,
    metadata_entities_json: String,
    pending_publish_variant_json: Option<String>,
    previous_keyboard_json: Option<String>,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl TryFrom<StoredReviewPost> for ReviewPost {
    type Error = anyhow::Error;

    fn try_from(stored: StoredReviewPost) -> Result<Self, Self::Error> {
        Ok(Self {
            chat_id: stored.chat_id,
            post_id: stored.post_id,
            source_url: stored.source_url,
            caption: RichText {
                text: stored.caption_text,
                entities: serde_json::from_str(&stored.caption_entities_json)
                    .context("could not deserialize review caption entities")?,
            },
            content_kind: serde_json::from_str(&stored.content_kind_json)
                .context("could not deserialize review content kind")?,
            caption_placement: if stored.caption_above_media {
                CaptionPlacement::AboveMedia
            } else {
                CaptionPlacement::BelowMedia
            },
            review_message_id: MessageId(stored.review_message_id),
            control_message_id: MessageId(stored.control_message_id),
            metadata: RichText {
                text: stored.metadata_text,
                entities: serde_json::from_str(&stored.metadata_entities_json)
                    .context("could not deserialize review metadata entities")?,
            },
            pending_publish_variant: stored
                .pending_publish_variant_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .context("could not deserialize pending publish variant")?,
            previous_keyboard: stored
                .previous_keyboard_json
                .as_deref()
                .map(serde_json::from_str)
                .transpose()
                .context("could not deserialize previous review keyboard")?,
            published_at: stored.published_at,
        })
    }
}

pub trait Recordable {
    fn id(&self) -> &str;
    fn title(&self) -> &str;
    fn subreddit(&self) -> &str;
}

impl ToSql for TopPostsTimePeriod {
    fn to_sql(&self) -> Result<rusqlite::types::ToSqlOutput<'_>, rusqlite::Error> {
        Ok(ToSqlOutput::Owned(Value::Text(self.to_string())))
    }
}

impl ToSql for PostType {
    fn to_sql(&self) -> Result<rusqlite::types::ToSqlOutput<'_>, rusqlite::Error> {
        Ok(ToSqlOutput::Owned(Value::Text(self.to_string())))
    }
}

impl FromSql for TopPostsTimePeriod {
    fn column_result(value: ValueRef) -> FromSqlResult<TopPostsTimePeriod> {
        let str = String::column_result(value)?;
        TopPostsTimePeriod::from_str(&str).map_err(|e| FromSqlError::Other(From::from(e)))
    }
}

impl FromSql for PostType {
    fn column_result(value: ValueRef) -> FromSqlResult<PostType> {
        let str = String::column_result(value)?;
        PostType::from_str(&str).map_err(|e| FromSqlError::Other(From::from(e)))
    }
}

impl TryFrom<&Row<'_>> for Subscription {
    type Error = rusqlite::Error;

    fn try_from(row: &Row<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            subreddit: row.get_unwrap("subreddit"),
            chat_id: row.get_unwrap("chat_id"),
            limit: row.get_unwrap("post_limit"),
            time: row.get_unwrap("time"),
            filter: row.get_unwrap("filter"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        reddit::PostType,
        types::{MediaKind, TelegramMediaFile},
    };
    use teloxide::types::{
        FileId, FileUniqueId, InlineKeyboardButton, MessageEntity, MessageEntityKind,
    };

    fn test_post() -> Post {
        Post {
            id: "v6nu75".into(),
            post_hint: Some("link".into()),
            subreddit: "absoluteunit".into(),
            title: "Tipping a cow to trim its hooves".into(),
            gallery_data: None,
            media_metadata: None,
            permalink: "/r/absoluteunit/comments/v6nu75/tipping_a_cow_to_trim_its_hooves/".into(),
            url: "https://i.imgur.com/Zt6f5mB.gifv".into(),
            post_type: PostType::Video,
        }
    }

    fn test_review_post() -> ReviewPost {
        ReviewPost {
            chat_id: 1,
            post_id: "v6nu75".to_owned(),
            source_url: "https://i.imgur.com/Zt6f5mB.gifv".to_owned(),
            caption: RichText {
                text: "Tipping a cow".to_owned(),
                entities: vec![MessageEntity::bold(0, 7)],
            },
            content_kind: ReviewContentKind::Media,
            caption_placement: CaptionPlacement::BelowMedia,
            review_message_id: MessageId(10),
            control_message_id: MessageId(10),
            metadata: RichText {
                text: "Source: https://example.com".to_owned(),
                entities: vec![MessageEntity::new(MessageEntityKind::Url, 8, 19)],
            },
            pending_publish_variant: None,
            previous_keyboard: None,
            published_at: None,
        }
    }

    fn migrated_db_with_post() -> Database {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        db.record_post_seen_with_current_time(1, &test_post())
            .unwrap();
        db
    }

    #[test]
    fn gallery_file_media_kinds_round_trip_in_delivery_order() {
        let db = migrated_db_with_post();
        db.add_telegram_file(
            "v6nu75",
            1,
            &FileId::from("photo-file"),
            &FileUniqueId::from("photo-unique"),
            MediaKind::Photo,
        )
        .unwrap();
        db.add_telegram_file(
            "v6nu75",
            1,
            &FileId::from("video-file"),
            &FileUniqueId::from("video-unique"),
            MediaKind::Video,
        )
        .unwrap();

        assert_eq!(
            db.get_telegram_files_for_post("v6nu75", 1).unwrap(),
            vec![
                TelegramMediaFile {
                    file_id: FileId::from("photo-file"),
                    kind: MediaKind::Photo,
                },
                TelegramMediaFile {
                    file_id: FileId::from("video-file"),
                    kind: MediaKind::Video,
                },
            ]
        );
    }

    #[test]
    fn test_db() {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        let post = test_post();

        assert!(!db.existing_posts_for_subreddit(1, "absoluteunit").unwrap());
        db.record_post_seen_with_current_time(1, &post).unwrap();
        assert!(db.is_post_seen(1, &post).unwrap());
        assert!(db.existing_posts_for_subreddit(1, "absoluteunit").unwrap());
    }

    #[test]
    fn test_db_subscribe() {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        let subscription_args = SubscriptionArgs {
            subreddit: "test".to_string(),
            limit: Some(1),
            time: Some(TopPostsTimePeriod::Week),
            filter: Some(PostType::Video),
        };
        db.subscribe(1, &subscription_args).unwrap();

        let subs = db.get_subscriptions_for_chat(1).unwrap();
        assert_eq!(
            subs,
            vec![Subscription {
                chat_id: 1,
                subreddit: "test".to_string(),
                limit: Some(1),
                time: Some(TopPostsTimePeriod::Week),
                filter: Some(PostType::Video),
            }]
        );
    }

    #[test]
    fn test_db_unsubscribe() {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        let subscription_args = SubscriptionArgs {
            subreddit: "test".to_string(),
            limit: Some(1),
            time: Some(TopPostsTimePeriod::Week),
            filter: Some(PostType::Video),
        };
        db.subscribe(1, &subscription_args).unwrap();
        let subs = db.get_subscriptions_for_chat(1).unwrap();
        assert_eq!(subs.len(), 1);
        let deleted = db.unsubscribe(1, "test").unwrap();
        assert_eq!(deleted, "test");
        let subs = db.get_subscriptions_for_chat(1).unwrap();
        assert_eq!(subs, vec![]);
    }

    #[test]
    fn test_db_unsubscribe_doesnt_delete_posts() {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        let subscription_args = SubscriptionArgs {
            subreddit: "test".to_string(),
            limit: Some(1),
            time: Some(TopPostsTimePeriod::Week),
            filter: Some(PostType::Video),
        };
        db.subscribe(1, &subscription_args).unwrap();
        let post = Post {
            id: "v6nu75".into(),
            post_hint: Some("link".into()),
            subreddit: "test".into(),
            title: "Tipping a cow to trim its hooves".into(),
            gallery_data: None,
            media_metadata: None,
            permalink: "/r/test/comments/v6nu75/tipping_a_cow_to_trim_its_hooves/".into(),
            url: "https://i.imgur.com/Zt6f5mB.gifv".into(),
            post_type: PostType::Video,
        };
        db.record_post_seen_with_current_time(1, &post).unwrap();
        assert!(db.is_post_seen(1, &post).unwrap());
        db.unsubscribe(1, "test").unwrap();
        assert!(db.is_post_seen(1, &post).unwrap());
    }

    #[test]
    fn review_post_round_trips_rich_text_and_keyboard() {
        let db = migrated_db_with_post();
        let mut review = test_review_post();
        review.pending_publish_variant = Some(PublishVariant::WithLink);
        review.previous_keyboard =
            Some(
                InlineKeyboardMarkup::default().append_row([InlineKeyboardButton::callback(
                    "Post",
                    r#"{"a":"p","p":"v6nu75"}"#,
                )]),
            );

        db.upsert_review_post(&review).unwrap();

        assert_eq!(
            db.get_review_post(1, "v6nu75").unwrap(),
            Some(review.clone())
        );
        assert_eq!(
            db.get_review_post_by_control_message(1, MessageId(10))
                .unwrap(),
            Some(review)
        );
    }

    #[test]
    fn review_post_does_not_require_a_local_post_cache_entry() {
        let config = Config::default();
        let mut db = Database::open(&config).unwrap();
        db.migrate().unwrap();
        let review = test_review_post();

        db.upsert_review_post(&review).unwrap();

        assert_eq!(
            db.get_review_post(review.chat_id, &review.post_id).unwrap(),
            Some(review)
        );
    }

    #[test]
    fn review_publish_state_transitions_are_persisted() {
        let db = migrated_db_with_post();
        let review = test_review_post();
        db.upsert_review_post(&review).unwrap();
        let keyboard =
            InlineKeyboardMarkup::default().append_row([InlineKeyboardButton::callback(
                "Post",
                r#"{"a":"p","p":"v6nu75"}"#,
            )]);

        db.begin_review_publish(
            review.chat_id,
            &review.post_id,
            PublishVariant::WithLink,
            Some(&keyboard),
        )
        .unwrap();
        let selected = db
            .get_review_post(review.chat_id, &review.post_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            selected.pending_publish_variant,
            Some(PublishVariant::WithLink)
        );
        assert_eq!(selected.previous_keyboard, Some(keyboard.clone()));

        db.clear_review_publish(review.chat_id, &review.post_id)
            .unwrap();
        let cleared = db
            .get_review_post(review.chat_id, &review.post_id)
            .unwrap()
            .unwrap();
        assert_eq!(cleared.pending_publish_variant, None);
        assert_eq!(cleared.previous_keyboard, None);

        db.begin_review_publish(
            review.chat_id,
            &review.post_id,
            PublishVariant::Caption,
            Some(&keyboard),
        )
        .unwrap();
        assert!(
            db.claim_review_publish(review.chat_id, &review.post_id)
                .unwrap()
        );
        assert!(
            !db.claim_review_publish(review.chat_id, &review.post_id)
                .unwrap()
        );
        db.mark_review_published(review.chat_id, &review.post_id)
            .unwrap();
        let published = db
            .get_review_post(review.chat_id, &review.post_id)
            .unwrap()
            .unwrap();
        assert!(published.published_at.is_some());
        assert_eq!(published.pending_publish_variant, None);
        assert_eq!(published.previous_keyboard, None);
        assert!(
            db.begin_review_publish(
                review.chat_id,
                &review.post_id,
                PublishVariant::WithoutCaption,
                Some(&keyboard),
            )
            .is_err()
        );
    }

    #[test]
    fn updating_review_caption_preserves_other_state() {
        let db = migrated_db_with_post();
        let review = test_review_post();
        db.upsert_review_post(&review).unwrap();
        let replacement = RichText {
            text: "A 🐄 caption".to_owned(),
            entities: vec![MessageEntity::italic(2, 2)],
        };

        db.update_review_caption(review.chat_id, &review.post_id, &replacement)
            .unwrap();

        let updated = db
            .get_review_post(review.chat_id, &review.post_id)
            .unwrap()
            .unwrap();
        assert_eq!(updated.caption, replacement);
        assert_eq!(updated.source_url, review.source_url);
        assert_eq!(updated.metadata, review.metadata);
    }

    #[test]
    fn caption_placement_defaults_below_and_persists_per_review_post() {
        let db = migrated_db_with_post();
        let review = test_review_post();
        db.upsert_review_post(&review).unwrap();

        assert_eq!(
            db.get_review_post(review.chat_id, &review.post_id)
                .unwrap()
                .unwrap()
                .caption_placement,
            CaptionPlacement::BelowMedia
        );

        db.update_caption_placement(
            review.chat_id,
            &review.post_id,
            CaptionPlacement::AboveMedia,
        )
        .unwrap();

        assert_eq!(
            db.get_review_post(review.chat_id, &review.post_id)
                .unwrap()
                .unwrap()
                .caption_placement,
            CaptionPlacement::AboveMedia
        );
        assert_eq!(db.active_media_review_posts().unwrap().len(), 1);

        db.begin_review_publish(
            review.chat_id,
            &review.post_id,
            PublishVariant::Caption,
            None,
        )
        .unwrap();
        assert!(db.active_media_review_posts().unwrap().is_empty());
        assert!(
            db.update_caption_placement(
                review.chat_id,
                &review.post_id,
                CaptionPlacement::BelowMedia,
            )
            .is_err()
        );
    }
}
