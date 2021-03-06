extern crate dirs;
extern crate rayon;
extern crate rust_bitbar;
extern crate sdk_cds;
extern crate serde_json;

use std::fs::File;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rayon::prelude::*;
use rust_bitbar::{Line, Plugin, SubMenu};
use sdk_cds::client::Client;
use sdk_cds::models;

const GREEN: &str = "#21BA45";
const RED: &str = "#FF4F60";
const BLUE: &str = "#4fa3e3";
const LIGHT_GREY: &str = "#8c96a5";
const ORANGE: &str = "#FE9A76";

fn main() {
    let mut file = File::open(format!(
        "{}/.cds.conf.json",
        dirs::home_dir()
            .expect("cannot get home directoy")
            .display()
    ))
    .expect("Could not find .cds.conf.json file");
    let mut data = String::new();
    file.read_to_string(&mut data).unwrap();

    let mut cds_client: Client =
        serde_json::from_str(&data).expect("cannot deserialize cds config");
    if cds_client.name == "" {
        cds_client.name = "CDS"
    }

    let mut plugin = Plugin::new();
    let me = cds_client.me().expect("cannot get current user infos");

    if me.admin {
        display_as_admin(&cds_client, &mut plugin);
    } else {
        display_as_user(&cds_client, &mut plugin);
    }

    plugin.render();
}

fn display_as_user(cds_client: &Client, plugin: &mut Plugin) {
    let mut status_line_text = format!("{}", cds_client.name);
    let mut status_line = Line::new(status_line_text.to_string());
    let broadcasts: Vec<models::Broadcast> = cds_client
        .broadcasts()
        .expect("cannot get broadcasts")
        .into_iter()
        .filter(|b| !b.read)
        .collect();
    let nb_broadcasts = broadcasts.len();
    let warning: bool = (&broadcasts)
        .into_iter()
        .any(|b| b.level == "warning".to_string());
    let mut sub_menu = SubMenu::new();

    let cds_url = cds_client.config().expect("cannot get config urls of CDS");
    let host = cds_client.host.to_string();
    let cds_ui_url = cds_url.get("url.ui").unwrap_or(&host);

    if nb_broadcasts > 0 {
        status_line_text = format!("{} 🔔{}", cds_client.name, nb_broadcasts);
        if warning {
            status_line.set_color(ORANGE);
        } else {
            status_line.set_color(BLUE);
        }

        let mut broadcast_title = Line::new("Broadcasts");
        broadcast_title.set_color(LIGHT_GREY);

        sub_menu.add_line(broadcast_title);
        for broadcast in broadcasts.into_iter() {
            let mut line = Line::new(format!("{}", broadcast.title));
            line.set_href(format!("{}/broadcast/{}", cds_ui_url, broadcast.id));
            if broadcast.level == "warning" {
                line.set_color(ORANGE);
            } else {
                line.set_color(BLUE);
            }
            sub_menu.add_line(line);
        }
        sub_menu.add_hr();
    }

    let bookmarks = cds_client.bookmarks().expect("cannot get bookmarks");
    if bookmarks.len() > 0 {
        let mut bookmarks_title = Line::new("Bookmarks");
        bookmarks_title.set_color(LIGHT_GREY);
        sub_menu.add_line(bookmarks_title);
    }
    let in_progress = add_workflow_run_status(cds_client, &cds_ui_url, &bookmarks, &mut sub_menu);

    if in_progress > 0 {
        status_line_text = format!("{} 🚧{}", status_line_text, in_progress);
    }
    status_line.set_text(status_line_text);

    plugin.set_status_line(status_line);
    plugin.set_sub_menu(sub_menu);
}

fn display_as_admin(cds_client: &Client, plugin: &mut Plugin) {
    let cds_status = cds_client.status().expect("cannot get cds status");
    let queue_count = cds_client
        .queue_count()
        .expect("cannot get cds queue count");

    let mut danger = false;
    let mut text: String = format!("{} ✔︎", cds_client.name);
    let mut status_line = Line::new(text.to_string());
    if let Some(lines) = cds_status.lines {
        danger = lines
            .iter()
            .any(|line| line.component == "Global/Status" && line.status == "AL");
    }

    let cds_url = cds_client.config().expect("cannot get config urls of CDS");
    let host = cds_client.host.to_string();
    let cds_ui_url = cds_url.get("url.ui").unwrap_or(&host);

    let mut sub_menu = SubMenu::new();
    let mut status_line_title = Line::new("CDS Status");
    status_line_title.set_color(LIGHT_GREY);
    sub_menu.add_line(status_line_title);
    let status_line_details = if danger {
        let mut curr_line = Line::new("Global/Status Alarm");
        curr_line.set_color(RED);
        curr_line.set_href(format!("{}/admin/services",cds_ui_url));
        curr_line
    } else {
        let mut curr_line = Line::new("Global/Status OK");
        curr_line.set_color(GREEN);
        curr_line.set_href(format!("{}/admin/services",cds_ui_url));
        curr_line
    };
    sub_menu.add_line(status_line_details);
    sub_menu.add_hr();

    if !danger {
        if queue_count.count > 50 {
            status_line.set_color(ORANGE);
        } else if queue_count.count > 100 {
            status_line.set_color(RED);
        } else {
            status_line.set_color(GREEN);
        }
    } else {
        text = format!("{} ✘", cds_client.name);
        status_line.set_color(RED);
    }

    let bookmarks = cds_client.bookmarks().expect("cannot get bookmarks");
    if bookmarks.len() > 0 {
        let mut bookmarks_title = Line::new("Bookmarks");
        bookmarks_title.set_color(LIGHT_GREY);
        sub_menu.add_line(bookmarks_title);
    }
    let in_progress = add_workflow_run_status(cds_client, &cds_ui_url, &bookmarks, &mut sub_menu);

    text = format!("{} 🚀{}", text, queue_count.count);
    if in_progress > 0 {
        text = format!("{} 🚧{}", text, in_progress);
    }
    status_line.set_text(text);
    plugin.set_status_line(status_line);

    plugin.set_sub_menu(sub_menu);
}

fn add_workflow_run_status(
    cds_client: &Client,
    cds_ui_url: &str,
    bookmarks: &Vec<models::Bookmark>,
    sub_menu: &mut SubMenu,
) -> usize {
    let in_progress = Arc::new(AtomicUsize::new(0));
    let lines: Vec<Line> = bookmarks
        .into_par_iter()
        .filter(|bookmark| bookmark._type == "workflow")
        .map(|bookmark| {
            let last_run_res = cds_client.last_run(&bookmark.key, &bookmark.workflow_name);
            // let in_progress = in_progress.clone();

            match last_run_res {
                Err(error) => {
                    if error.status == 404u16 {
                        return Line::new(format!(
                            "{}/{} #0.0 never built",
                            bookmark.key, bookmark.workflow_name
                        ));
                    } else {
                        return Line::new(format!(
                            "{}/{} ERROR : {:?}",
                            bookmark.key, bookmark.workflow_name, error
                        ));
                    }
                }
                Ok(last_run) => {
                    let mut workflow_line = Line::new(format!(
                        "{}/{} #{}.{}",
                        bookmark.key, bookmark.workflow_name, last_run.num, last_run.last_subnumber
                    ));
                    match last_run.status.as_ref() {
                        "Success" => {
                            workflow_line.set_color(GREEN);
                        }
                        "Building" | "Checking" | "Waiting" => {
                            workflow_line.set_color(BLUE);
                            in_progress
                                .store(in_progress.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
                        }
                        "Skipped" | "Never Built" => {
                            workflow_line.set_color("grey");
                        }
                        _ => {
                            workflow_line.set_color(RED);
                        }
                    };
                    workflow_line.set_href(format!(
                        "{}/project/{}/workflow/{}/run/{}",
                        cds_ui_url, bookmark.key, bookmark.workflow_name, last_run.num
                    ));
                    return workflow_line;
                }
            }
        })
        .collect();

    for line in lines {
        sub_menu.add_line(line);
    }

    in_progress.load(Ordering::Relaxed)
}
