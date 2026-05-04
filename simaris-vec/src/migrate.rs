//! sqlite -> arrow -> lance migration.
//!
//! Schema:
//!
//! Lance dataset `units`:
//!   id          string  (uuidv7)
//!   content     string
//!   type        string
//!   source      string
//!   confidence  float32
//!   verified    bool
//!   archived    bool
//!   tags_json   string  (raw JSON array, deferred parsing)
//!   conditions_json string
//!   created     string
//!   updated     string
//!   embedding   fixed-size-list<float32, dim>  (placeholder zeros for now)
//!
//! Lance dataset `links`:
//!   from_id        string
//!   to_id          string
//!   relationship   string
//!
//! Side tables stored as separate Lance datasets in same parent dir:
//!   slugs   (slug, unit_id, created)
//!   marks   (id, unit_id, kind, created)
//!   inbox   (id, content, source, created)
//!
//! Tantivy index on units(content + tags + type + source) for hybrid query.

use anyhow::{Context, Result};
use arrow_array::builder::Float32Builder;
use arrow_array::{
    BooleanArray, FixedSizeListArray, Float32Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use lance::Dataset;
use lance::dataset::WriteParams;
use rusqlite::Connection;
use std::path::Path;
use std::sync::Arc;

use tantivy::schema::{STORED, STRING, Schema as TSchema, TEXT};
use tantivy::{Index, doc};

pub async fn run(
    sqlite: &Path,
    lance_dir: &Path,
    tantivy_dir: &Path,
    dim: usize,
    model: &str,
) -> Result<()> {
    if lance_dir.exists() {
        std::fs::remove_dir_all(lance_dir).ok();
    }
    if tantivy_dir.exists() {
        std::fs::remove_dir_all(tantivy_dir).ok();
    }
    std::fs::create_dir_all(lance_dir)?;
    std::fs::create_dir_all(tantivy_dir)?;

    let conn = Connection::open(sqlite)?;

    let units_path = lance_dir.join("units.lance");
    let links_path = lance_dir.join("links.lance");
    let slugs_path = lance_dir.join("slugs.lance");
    let marks_path = lance_dir.join("marks.lance");
    let inbox_path = lance_dir.join("inbox.lance");

    let n_units = migrate_units(&conn, &units_path, dim, model).await?;
    let n_links = migrate_links(&conn, &links_path).await?;
    let n_slugs = migrate_slugs(&conn, &slugs_path).await?;
    let n_marks = migrate_marks(&conn, &marks_path).await?;
    let n_inbox = migrate_inbox(&conn, &inbox_path).await?;

    let n_indexed = build_tantivy(&conn, tantivy_dir)?;

    println!("migrate.units: {n_units}");
    println!("migrate.links: {n_links}");
    println!("migrate.slugs: {n_slugs}");
    println!("migrate.marks: {n_marks}");
    println!("migrate.inbox: {n_inbox}");
    println!("tantivy.indexed: {n_indexed}");
    println!("embedding.dim: {dim}");
    println!("embedding.model: {model} (placeholder zeros)");
    Ok(())
}

fn units_schema(dim: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("content", DataType::Utf8, false),
        Field::new("type", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("confidence", DataType::Float32, false),
        Field::new("verified", DataType::Boolean, false),
        Field::new("archived", DataType::Boolean, false),
        Field::new("tags_json", DataType::Utf8, false),
        Field::new("conditions_json", DataType::Utf8, false),
        Field::new("created", DataType::Utf8, false),
        Field::new("updated", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dim as i32,
            ),
            false,
        ),
    ]))
}

async fn migrate_units(conn: &Connection, out: &Path, dim: usize, _model: &str) -> Result<usize> {
    let schema = units_schema(dim);

    let mut stmt = conn.prepare(
        "select id, content, type, source, confidence, verified, archived,
                tags, conditions, created, updated from units order by id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, f64>(4)? as f32,
            r.get::<_, i64>(5)? != 0,
            r.get::<_, i64>(6)? != 0,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, String>(9)?,
            r.get::<_, String>(10)?,
        ))
    })?;

    let mut ids = Vec::new();
    let mut contents = Vec::new();
    let mut types = Vec::new();
    let mut sources = Vec::new();
    let mut confs = Vec::new();
    let mut verifs = Vec::new();
    let mut archs = Vec::new();
    let mut tagsj = Vec::new();
    let mut condj = Vec::new();
    let mut crts = Vec::new();
    let mut upds = Vec::new();
    for row in rows {
        let r = row?;
        ids.push(r.0);
        contents.push(r.1);
        types.push(r.2);
        sources.push(r.3);
        confs.push(r.4);
        verifs.push(r.5);
        archs.push(r.6);
        tagsj.push(r.7);
        condj.push(r.8);
        crts.push(r.9);
        upds.push(r.10);
    }
    let n = ids.len();

    // build placeholder embedding column (zero vectors).
    let total = n * dim;
    let mut emb_builder = Float32Builder::with_capacity(total);
    for _ in 0..total {
        emb_builder.append_value(0.0);
    }
    let values_arr = emb_builder.finish();
    let emb_field = Arc::new(Field::new("item", DataType::Float32, true));
    let emb = FixedSizeListArray::try_new(emb_field, dim as i32, Arc::new(values_arr), None)?;

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(ids)),
            Arc::new(StringArray::from(contents)),
            Arc::new(StringArray::from(types)),
            Arc::new(StringArray::from(sources)),
            Arc::new(Float32Array::from(confs)),
            Arc::new(BooleanArray::from(verifs)),
            Arc::new(BooleanArray::from(archs)),
            Arc::new(StringArray::from(tagsj)),
            Arc::new(StringArray::from(condj)),
            Arc::new(StringArray::from(crts)),
            Arc::new(StringArray::from(upds)),
            Arc::new(emb),
        ],
    )?;

    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema.clone());
    let uri = out.to_str().context("path utf8")?;
    Dataset::write(reader, uri, Some(WriteParams::default())).await?;
    Ok(n)
}

fn flat_schema(fields: Vec<(&str, DataType)>) -> Arc<Schema> {
    Arc::new(Schema::new(
        fields
            .into_iter()
            .map(|(n, t)| Field::new(n, t, false))
            .collect::<Vec<_>>(),
    ))
}

async fn migrate_links(conn: &Connection, out: &Path) -> Result<usize> {
    let schema = flat_schema(vec![
        ("from_id", DataType::Utf8),
        ("to_id", DataType::Utf8),
        ("relationship", DataType::Utf8),
    ]);
    let mut stmt = conn.prepare("select from_id, to_id, relationship from links")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    for row in rows {
        let (x, y, z) = row?;
        a.push(x);
        b.push(y);
        c.push(z);
    }
    let n = a.len();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(a)),
            Arc::new(StringArray::from(b)),
            Arc::new(StringArray::from(c)),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
    let uri = out.to_str().context("utf8")?;
    Dataset::write(reader, uri, Some(WriteParams::default())).await?;
    Ok(n)
}

async fn migrate_slugs(conn: &Connection, out: &Path) -> Result<usize> {
    let schema = flat_schema(vec![
        ("slug", DataType::Utf8),
        ("unit_id", DataType::Utf8),
        ("created", DataType::Utf8),
    ]);
    let mut stmt = conn.prepare("select slug, unit_id, created from slugs")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        let (x, y, z) = row?;
        a.push(x);
        b.push(y);
        c.push(z);
    }
    let n = a.len();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(a)),
            Arc::new(StringArray::from(b)),
            Arc::new(StringArray::from(c)),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
    let uri = out.to_str().context("utf8")?;
    Dataset::write(reader, uri, Some(WriteParams::default())).await?;
    Ok(n)
}

async fn migrate_marks(conn: &Connection, out: &Path) -> Result<usize> {
    let schema = flat_schema(vec![
        ("id", DataType::Utf8),
        ("unit_id", DataType::Utf8),
        ("kind", DataType::Utf8),
        ("created", DataType::Utf8),
    ]);
    let mut stmt = conn.prepare("select id, unit_id, kind, created from marks")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let (mut a, mut b, mut c, mut d) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        let (w, x, y, z) = row?;
        a.push(w);
        b.push(x);
        c.push(y);
        d.push(z);
    }
    let n = a.len();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(a)),
            Arc::new(StringArray::from(b)),
            Arc::new(StringArray::from(c)),
            Arc::new(StringArray::from(d)),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
    let uri = out.to_str().context("utf8")?;
    Dataset::write(reader, uri, Some(WriteParams::default())).await?;
    Ok(n)
}

async fn migrate_inbox(conn: &Connection, out: &Path) -> Result<usize> {
    let schema = flat_schema(vec![
        ("id", DataType::Utf8),
        ("content", DataType::Utf8),
        ("source", DataType::Utf8),
        ("created", DataType::Utf8),
    ]);
    let mut stmt = conn.prepare("select id, content, source, created from inbox")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let (mut a, mut b, mut c, mut d) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for row in rows {
        let (w, x, y, z) = row?;
        a.push(w);
        b.push(x);
        c.push(y);
        d.push(z);
    }
    let n = a.len();
    if n == 0 {
        // arrow's empty batch path is fine
    }
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(a)),
            Arc::new(StringArray::from(b)),
            Arc::new(StringArray::from(c)),
            Arc::new(StringArray::from(d)),
        ],
    )?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)].into_iter(), schema);
    let uri = out.to_str().context("utf8")?;
    Dataset::write(reader, uri, Some(WriteParams::default())).await?;
    Ok(n)
}

fn build_tantivy(conn: &Connection, dir: &Path) -> Result<usize> {
    let mut sb = TSchema::builder();
    let f_id = sb.add_text_field("id", STRING | STORED);
    let f_content = sb.add_text_field("content", TEXT | STORED);
    let f_tags = sb.add_text_field("tags", TEXT | STORED);
    let f_type = sb.add_text_field("type", STRING | STORED);
    let f_source = sb.add_text_field("source", STRING | STORED);
    let schema = sb.build();

    let index = Index::create_in_dir(dir, schema)?;
    let mut writer = index.writer(50_000_000)?;

    let mut stmt =
        conn.prepare("select id, content, tags, type, source from units where archived = 0")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    let mut n = 0;
    for row in rows {
        let (id, content, tags, ty, src) = row?;
        writer.add_document(doc!(
            f_id => id,
            f_content => content,
            f_tags => tags,
            f_type => ty,
            f_source => src,
        ))?;
        n += 1;
    }
    writer.commit()?;
    Ok(n)
}
