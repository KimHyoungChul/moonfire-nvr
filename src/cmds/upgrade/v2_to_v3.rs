// This file is part of Moonfire NVR, a security camera digital video recorder.
// Copyright (C) 2018 Scott Lamb <slamb@slamb.org>
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// In addition, as a special exception, the copyright holders give
// permission to link the code of portions of this program with the
// OpenSSL library under certain conditions as described in each
// individual source file, and distribute linked combinations including
// the two.
//
// You must obey the GNU General Public License in all respects for all
// of the code used other than OpenSSL. If you modify file(s) with this
// exception, you may extend this exception to your version of the
// file(s), but you are not obligated to do so. If you do not wish to do
// so, delete this exception statement from your version. If you delete
// this exception statement from all source files in the program, then
// also delete it here.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <http://www.gnu.org/licenses/>.

/// Upgrades a version 2 schema to a version 3 schema.

use db::{self, dir, FromSqlUuid};
use failure::Error;
use libc;
use std::io::{self, Write};
use std::mem;
//use std::rc::Rc;
use rusqlite;
use uuid::Uuid;

pub struct U;

pub fn new<'a>(_args: &'a super::Args) -> Result<Box<super::Upgrader + 'a>, Error> {
    Ok(Box::new(U))
}

/*fn build_stream_to_dir(dir: &dir::Fd, tx: &rusqlite::Transaction)
                           -> Result<Vec<Option<Rc<dir::Fd>>>, Error> {
    let mut v = Vec::new();
    let max_id: u32 = tx.query_row("select max(id) from stream", &[], |r| r.get_checked(0))??;
    v.resize((max_id + 1) as usize, None);
    let mut stmt = tx.prepare(r#"
        select
          stream.id,
          camera.uuid,
          stream.type
        from
          camera join stream on (camera.id = stream.camera_id)
    "#)?;
    let mut rows = stmt.query(&[])?;
    while let Some(row) = rows.next() {
        let row = row?;
        let id: i32 = row.get_checked(0)?;
        let uuid: FromSqlUuid = row.get_checked(1)?;
        let type_: String = row.get_checked(2)?;
        v[id as usize] =
            Some(Rc::new(dir::Fd::open(Some(dir), &format!("{}-{}", uuid.0, type_), true)?));
    }
    Ok(v)
}*/

/// Gets a pathname for a sample file suitable for passing to open or unlink.
fn get_uuid_pathname(uuid: Uuid) -> [libc::c_char; 37] {
    let mut buf = [0u8; 37];
    write!(&mut buf[..36], "{}", uuid.hyphenated()).expect("can't format uuid to pathname buf");

    // libc::c_char seems to be i8 on some platforms (Linux/arm) and u8 on others (Linux/amd64).
    unsafe { mem::transmute::<[u8; 37], [libc::c_char; 37]>(buf) }
}

fn get_id_pathname(id: db::CompositeId) -> [libc::c_char; 17] {
    let mut buf = [0u8; 17];
    write!(&mut buf[..16], "{:016x}", id.0).expect("can't format id to pathname buf");
    unsafe { mem::transmute::<[u8; 17], [libc::c_char; 17]>(buf) }
}

impl super::Upgrader for U {
    fn in_tx(&mut self, tx: &rusqlite::Transaction) -> Result<(), Error> {
        /*let (meta, path) = tx.query_row(r#"
            select
              meta.uuid,
              dir.path,
              dir.uuid,
              dir.last_complete_open_id,
              open.uuid
            from
              meta cross join sample_file_dir dir
              join open on (dir.last_complete_open_id = open.id)
        "#, |row| -> Result<_, Error> {
            let mut meta = DirMeta::new();
            let db_uuid: FromSqlUuid = row.get_checked(0)?;
            let path: String = row.get_checked(1)?;
            let dir_uuid: FromSqlUuid = row.get_checked(2)?;
            let open_uuid: FromSqlUuid = row.get_checked(4)?;
            meta.db_uuid.extend_from_slice(&db_uuid.0.as_bytes()[..]);
            meta.dir_uuid.extend_from_slice(&dir_uuid.0.as_bytes()[..]);
            let open = meta.mut_last_complete_open();
            open.id = row.get_checked(3)?;
            open.uuid.extend_from_slice(&open_uuid.0.as_bytes()[..]);
            Ok((meta, path))
        })??;*/

        let path: String = tx.query_row(r#"
            select path from sample_file_dir
        "#, &[], |row| { row.get_checked(0) })??;

        // Build map of stream -> dirname.
        let d = dir::Fd::open(None, &path, false)?;
        //let stream_to_dir = build_stream_to_dir(&d, tx)?;

        let mut stmt = tx.prepare(r#"
            select
              composite_id,
              sample_file_uuid
            from
              recording_playback
        "#)?;
        let mut rows = stmt.query(&[])?;
        while let Some(row) = rows.next() {
            let row = row?;
            let id = db::CompositeId(row.get_checked(0)?);
            let sample_file_uuid: FromSqlUuid = row.get_checked(1)?;
            let from_path = get_uuid_pathname(sample_file_uuid.0);
            let to_path = get_id_pathname(id);
            //let to_dir: &dir::Fd = stream_to_dir[stream_id as usize].as_ref().unwrap();
            let r = unsafe { dir::renameat(&d, from_path.as_ptr(), &d, to_path.as_ptr()) };
            if let Err(e) = r {
                if e.kind() == io::ErrorKind::NotFound {
                    continue;  // assume it was already moved.
                }
                Err(e)?;
            }
        }

        // These create statements match the schema.sql when version 3 was the latest.
        tx.execute_batch(r#"
            alter table recording_playback rename to old_recording_playback;
            create table recording_playback (
              composite_id integer primary key references recording (composite_id),
              open_id integer not null references open (id),
              sample_file_sha1 blob not null check (length(sample_file_sha1) = 20),
              video_index blob not null check (length(video_index) > 0)
            );
            insert into recording_playback
            select
              p.composite_id,
              o.id,
              p.sample_file_sha1,
              p.video_index
            from
              old_recording_playback p cross join open o;
            drop table old_recording_playback;
        "#)?;
        Ok(())
    }
}