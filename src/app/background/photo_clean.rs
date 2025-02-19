// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

use relm4::prelude::*;
use relm4::Worker;
use rayon::prelude::*;
use anyhow::*;

use tracing::{event, Level};

#[derive(Debug)]
pub enum PhotoCleanInput {
    Start,
}

#[derive(Debug)]
pub enum PhotoCleanOutput {
    // Thumbnail generation has started for a given number of images.
    Started,

    // Thumbnail generation has completed
    Completed,

}

pub struct PhotoClean {
    // Danger! Don't hold the repo mutex for too long as it blocks viewing images.
    repo: fotema_core::photo::Repository,
}

impl PhotoClean {

    fn cleanup(&mut self, sender: &ComponentSender<Self>) -> Result<()> {

        let start = std::time::Instant::now();

        // Scrub pics from database if they no longer exist on the file system.
        let pics: Vec<fotema_core::photo::model::Picture> = self.repo.all()?;

        let count = pics.len();

        // Short-circuit before sending progress messages to stop
        // banner from appearing and disappearing.
        if count == 0 {
            let _ = sender.output(PhotoCleanOutput::Completed);
            return Ok(());
        }

        if let Err(e) = sender.output(PhotoCleanOutput::Started){
            event!(Level::ERROR, "Failed sending cleanup started: {:?}", e);
        }

        pics.par_iter()
            .for_each(|pic| {
                if !pic.path.exists() {
                    let result = self.repo.clone().remove(pic.picture_id);
                    if let Err(e) = result {
                        event!(Level::ERROR, "Failed remove {}: {:?}", pic.picture_id, e);
                    } else {
                        event!(Level::INFO, "Removed {}", pic.picture_id);
                    }
                }
            });

        event!(Level::INFO, "Cleaned {} photos in {} seconds.", count, start.elapsed().as_secs());

        if let Err(e) = sender.output(PhotoCleanOutput::Completed) {
            event!(Level::ERROR, "Failed sending PhotoCleanOutput::Completed: {:?}", e);
        }

        Ok(())
    }
}

impl Worker for PhotoClean {
    type Init = fotema_core::photo::Repository;
    type Input = PhotoCleanInput;
    type Output = PhotoCleanOutput;

    fn init(repo: Self::Init, _sender: ComponentSender<Self>) -> Self {
        Self { repo }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            PhotoCleanInput::Start => {
                event!(Level::INFO, "Cleaning photos...");

                if let Err(e) = self.cleanup(&sender) {
                    event!(Level::ERROR, "Failed to clean photos: {}", e);
                }
            }
        };
    }
}
