// SPDX-FileCopyrightText: © 2024 David Bliss
//
// SPDX-License-Identifier: GPL-3.0-or-later

use gtk::prelude::OrientableExt;
use fotema_core;

use fotema_core::visual::model::PictureOrientation;
use strum::IntoEnumIterator;

use itertools::Itertools;
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::gdk_pixbuf;
use relm4::gtk::prelude::FrameExt;
use relm4::gtk::prelude::WidgetExt;
use relm4::typed_view::grid::{RelmGridItem, TypedGridView};
use relm4::*;
use relm4::binding::*;

use fotema_core::Year;
use fotema_core::YearMonth;
use std::path;
use std::sync::Arc;

use crate::adaptive;
use crate::app::SharedState;
use crate::app::ActiveView;
use crate::app::ViewName;
use crate::fl;

use tracing::{event, Level};

const NARROW_EDGE_LENGTH: i32 = 170;
const WIDE_EDGE_LENGTH: i32 = 200;

#[derive(Debug)]
struct PhotoGridItem {
    picture: Arc<fotema_core::visual::Visual>,

    // Length of thumbnail edge to allow for resizing when layout changes.
    edge_length: I32Binding,
}

struct Widgets {
    picture: gtk::Picture,
    label: gtk::Label,
}
#[derive(Debug)]
pub enum MonthsAlbumInput {
    Activate,

    /// A month has been selected in the grid view
    MonthSelected(u32), // WARN this is an index into a Vec, not a month

    /// Scroll to first photo of year
    GoToYear(Year),

    // Reload photos from database
    Refresh,

    // Adapt to layout
    Adapt(adaptive::Layout),
}

#[derive(Debug)]
pub enum MonthsAlbumOutput {
    MonthSelected(YearMonth),
}

impl RelmGridItem for PhotoGridItem {
    type Root = gtk::AspectFrame;
    type Widgets = Widgets;

    fn setup(_item: &gtk::ListItem) -> (Self::Root, Self::Widgets) {
        relm4::view! {
           root = gtk::AspectFrame {
                gtk::Frame {
                    gtk::Overlay {
                        add_overlay =  &gtk::Frame {
                            set_halign: gtk::Align::Start,
                            set_valign: gtk::Align::Start,
                            set_margin_start: 8,
                            set_margin_top: 8,
                            add_css_class: "photo-grid-month-frame",

                            #[wrap(Some)]
                            #[name(label)]
                            set_child = &gtk::Label {
                                add_css_class: "photo-grid-month-label",
                            },
                        },

                        #[wrap(Some)]
                        #[name(picture)]
                        set_child = &gtk::Picture {
                            set_can_shrink: true,
                            set_width_request: 170,
                            set_height_request: 170,
                        }
                    }
                }
            }
        }

        let widgets = Widgets { picture, label };

        (root, widgets)
    }

    fn bind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        let ym = self.picture.year_month();

        widgets.picture.add_write_only_binding(&self.edge_length, "width-request");
        widgets.picture.add_write_only_binding(&self.edge_length, "height-request");

        widgets
            .label
            .set_label(&fl!("month-thumbnail-label",
                month = ym.month.number_from_month(),
                year = ym.year.to_string()) // Should we convert to string?
            );

        if self.picture.thumbnail_path.as_ref().is_some_and(|x| x.exists()) {
            widgets
                .picture
                .set_filename(self.picture.thumbnail_path.clone());

            // Add CSS class for orientation
            let orientation = self.picture.thumbnail_orientation();
            widgets.picture.add_css_class(orientation.as_ref());
        } else {
            let pb = gdk_pixbuf::Pixbuf::from_resource_at_scale(
                "/app/fotema/Fotema/icons/scalable/actions/image-missing-symbolic.svg",
                200, 200, true
            ).unwrap();
            let img = gdk::Texture::for_pixbuf(&pb);
            widgets.picture.set_paintable(Some(&img));
        }
    }

    fn unbind(&mut self, widgets: &mut Self::Widgets, _root: &mut Self::Root) {
        widgets.picture.set_filename(None::<&path::Path>);
        // clear orientation transformation css classes
        for orient in PictureOrientation::iter() {
            widgets.picture.remove_css_class(orient.as_ref());
        }
    }
}

pub struct MonthsAlbum {
    state: SharedState,
    active_view: ActiveView,
    photo_grid: TypedGridView<PhotoGridItem, gtk::SingleSelection>,
    edge_length: I32Binding,
}

#[relm4::component(pub)]
impl SimpleComponent for MonthsAlbum {
    type Init = (SharedState, ActiveView);
    type Input = MonthsAlbumInput;
    type Output = MonthsAlbumOutput;

    view! {
        gtk::ScrolledWindow {
            set_vexpand: true,

            #[local_ref]
            photo_grid_view -> gtk::GridView {
                set_orientation: gtk::Orientation::Vertical,
                set_single_click_activate: true,

                connect_activate[sender] => move |_, idx| {
                    sender.input(MonthsAlbumInput::MonthSelected(idx))
                },
            },
        }
    }

    fn init(
        (state, active_view): Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let photo_grid = TypedGridView::new();

        let model = MonthsAlbum {
            state,
            active_view,
            photo_grid,
            edge_length: I32Binding::new(NARROW_EDGE_LENGTH),
        };

        let photo_grid_view = &model.photo_grid.view;

        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        match msg {
            MonthsAlbumInput::Activate => {
                *self.active_view.write() = ViewName::Month;
                if self.photo_grid.is_empty() {
                    self.refresh();
                }
            }
            MonthsAlbumInput::Refresh => {
                if *self.active_view.read() == ViewName::Month {
                    self.refresh();
                } else {
                    self.photo_grid.clear();
                }
            }
            MonthsAlbumInput::MonthSelected(index) => {
                if let Some(item) = self.photo_grid.get(index) {
                    let ym = item.borrow().picture.year_month();
                    event!(Level::DEBUG, "index {} has year_month {}", index, ym);
                    let _ = sender.output(MonthsAlbumOutput::MonthSelected(ym));
                }
            }
            MonthsAlbumInput::GoToYear(year) => {
                event!(Level::INFO, "Showing for year: {}", year);
                let index_opt = self
                    .photo_grid
                    .find(|p| p.picture.year_month().year == year);
                event!(Level::DEBUG, "Found: {:?}", index_opt);
                if let Some(index) = index_opt {
                    let flags = gtk::ListScrollFlags::SELECT;
                    event!(Level::DEBUG, "Scrolling to {}", index);
                    self.photo_grid.view.scroll_to(index, flags, None);
                }
            }
            MonthsAlbumInput::Adapt(adaptive::Layout::Narrow) => {
                self.edge_length.set_value(NARROW_EDGE_LENGTH);
            },
            MonthsAlbumInput::Adapt(adaptive::Layout::Wide) => {
                self.edge_length.set_value(WIDE_EDGE_LENGTH);
            },
        }
    }
}

impl MonthsAlbum {
    fn refresh(&mut self) {
        let all_pictures = {
            let data = self.state.read();
            data
                .iter()
                .dedup_by(|x, y| x.year_month() == y.year_month())
                .map(|picture| PhotoGridItem {
                    picture: picture.clone(),
                    edge_length: self.edge_length.clone(),
                })
                .collect::<Vec<PhotoGridItem>>()
        };

        self.photo_grid.clear();
        self.photo_grid.extend_from_iter(all_pictures);

        if !self.photo_grid.is_empty() {
            self.photo_grid.view.scroll_to(
                self.photo_grid.len() - 1,
                gtk::ListScrollFlags::SELECT,
                None,
            );
        }
    }
}

