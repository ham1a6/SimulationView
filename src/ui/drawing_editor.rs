//! 図形の作成・編集パネル。`terrain::draw_tool::DrawToolState`(context)を操作する。
//! ツールを選んで地図(`TerrainView`)をクリックすると図形が置かれ、一覧に載る。一覧で選んだ図形は
//! 地図上で黄色い枠になり、位置・大きさ・高度・見た目を数値で編集できる。
//!
//! モーダルにすると地図をクリックできなくなるので、既定(モーダル)の`FloatingPanel`には入れない。
//! 非モーダルの`FloatingPanel`(`modal=false`、`draggable`で移動可)か、`TabbedPanel`のタブなどの
//! 通常のパネルの中身として置く。

use leptos::prelude::*;

use crate::terrain::draw_tool::{DrawToolState, ToolKind, UserShape};
use crate::terrain::drawing::{Altitude, Color, DrawingId, Position, Shape, Style};
use crate::ui::context_menu::{ContextMenuState, MenuItem};

// ---------------------------------------------------------------------------------------------
// 入力欄の部品
// ---------------------------------------------------------------------------------------------

/// 数値を入力欄に出す形(小数`digits`桁で丸め、末尾の0は付けない)。
fn fmt_num(v: f64, digits: i32) -> String {
    let scale = 10f64.powi(digits);
    format!("{}", (v * scale).round() / scale)
}

/// 数値の入力欄。値の確定(フォーカスを外す・Enter)で`set`を呼ぶ。数値として読めない入力は無視する。
fn num_field(
    label: &'static str,
    unit: &'static str,
    get: impl Fn() -> f64 + Send + Sync + 'static,
    set: impl Fn(f64) + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <label class="drawing-field">
            <span>{format!("{label}({unit})")}</span>
            <input
                type="number"
                step="any"
                prop:value=move || fmt_num(get(), if unit == "°" { 6 } else { 3 })
                on:change=move |ev| {
                    if let Ok(v) = event_target_value(&ev).trim().parse::<f64>() {
                        if v.is_finite() {
                            set(v);
                        }
                    }
                }
            />
        </label>
    }
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn color_hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", to_u8(c.r), to_u8(c.g), to_u8(c.b))
}

fn hex_rgb(s: &str) -> Option<(f32, f32, f32)> {
    let s = s.strip_prefix('#')?;
    if s.len() != 6 || !s.is_ascii() {
        return None;
    }
    let channel = |i: usize| {
        u8::from_str_radix(&s[i..i + 2], 16)
            .ok()
            .map(|v| f32::from(v) / 255.0)
    };
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// 見た目(塗り・線)の入力欄。`with_fill`がfalseなら塗りの欄を出さない(折れ線)。
fn style_fields(
    get: impl Fn() -> Style + Copy + Send + Sync + 'static,
    set: impl Fn(Style) + Copy + Send + Sync + 'static,
    with_fill: bool,
) -> impl IntoView {
    let default_fill = Style::default().fill.unwrap_or(Color::WHITE);
    let fill_row = with_fill.then(|| {
        view! {
            <div class="drawing-row">
                <label class="drawing-check">
                    <input
                        type="checkbox"
                        prop:checked=move || get().fill.is_some()
                        on:change=move |ev| {
                            let mut style = get();
                            style.fill = event_target_checked(&ev).then_some(default_fill);
                            set(style);
                        }
                    />
                    "塗り"
                </label>
                <input
                    type="color"
                    prop:value=move || color_hex(get().fill.unwrap_or(default_fill))
                    disabled=move || get().fill.is_none()
                    on:input=move |ev| {
                        if let Some((r, g, b)) = hex_rgb(&event_target_value(&ev)) {
                            let mut style = get();
                            let a = style.fill.map_or(default_fill.a, |c| c.a);
                            style.fill = Some(Color::rgba(r, g, b, a));
                            set(style);
                        }
                    }
                />
                <label class="drawing-inline">
                    "不透明度"
                    <input
                        type="range"
                        min="0.05"
                        max="1"
                        step="0.05"
                        prop:value=move || get().fill.map_or(default_fill.a, |c| c.a).to_string()
                        disabled=move || get().fill.is_none()
                        on:input=move |ev| {
                            if let (Ok(a), Some(c)) = (event_target_value(&ev).parse::<f32>(), get().fill) {
                                let mut style = get();
                                style.fill = Some(Color::rgba(c.r, c.g, c.b, a));
                                set(style);
                            }
                        }
                    />
                </label>
            </div>
        }
    });
    let default_stroke = Style::default().stroke.unwrap_or(Color::WHITE);
    let stroke_check = with_fill.then(|| {
        view! {
            <input
                type="checkbox"
                prop:checked=move || get().stroke.is_some()
                on:change=move |ev| {
                    let mut style = get();
                    style.stroke = event_target_checked(&ev).then_some(default_stroke);
                    set(style);
                }
            />
        }
    });
    view! {
        <div class="drawing-style">
            {fill_row}
            <div class="drawing-row">
                <label class="drawing-check">{stroke_check} "線"</label>
                <input
                    type="color"
                    prop:value=move || color_hex(get().stroke.unwrap_or(default_stroke))
                    disabled=move || get().stroke.is_none()
                    on:input=move |ev| {
                        if let Some((r, g, b)) = hex_rgb(&event_target_value(&ev)) {
                            let mut style = get();
                            let a = style.stroke.map_or(1.0, |c| c.a);
                            style.stroke = Some(Color::rgba(r, g, b, a));
                            set(style);
                        }
                    }
                />
                {num_field(
                    "太さ",
                    "px",
                    move || f64::from(get().stroke_width_px),
                    move |v| {
                        let mut style = get();
                        style.stroke_width_px = v.clamp(0.5, 50.0) as f32;
                        set(style);
                    },
                )}
            </div>
        </div>
    }
}

fn altitude_value(a: Altitude) -> f64 {
    match a {
        Altitude::Msl(v) | Altitude::AboveGround(v) => v,
    }
}

/// 高度の入力欄(基準の選択+値)。
fn altitude_fields(
    get: impl Fn() -> Altitude + Copy + Send + Sync + 'static,
    set: impl Fn(Altitude) + Copy + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div class="drawing-row">
            <label class="drawing-field">
                <span>"高度の基準"</span>
                <select on:change=move |ev| {
                    let v = altitude_value(get());
                    set(if event_target_value(&ev) == "msl" { Altitude::Msl(v) } else { Altitude::AboveGround(v) });
                }>
                    <option value="agl" prop:selected=move || matches!(get(), Altitude::AboveGround(_))>
                        "地表から(地形に沿う)"
                    </option>
                    <option value="msl" prop:selected=move || matches!(get(), Altitude::Msl(_))>
                        "海抜"
                    </option>
                </select>
            </label>
            {num_field(
                "高度",
                "m",
                move || altitude_value(get()),
                move |v| {
                    set(match get() {
                        Altitude::Msl(_) => Altitude::Msl(v),
                        Altitude::AboveGround(_) => Altitude::AboveGround(v),
                    })
                },
            )}
        </div>
    }
}

// ---------------------------------------------------------------------------------------------
// 図形の数値パラメータ(図形の種類ごとに、どの値を編集できるか)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Param {
    Radius,
    Width,
    Height,
    Rotation,
    StartDeg,
    EndDeg,
    SizeEw,
    SizeNs,
    SizeUp,
    Heading,
}

/// 図形の種類ごとの編集項目(項目, ラベル, 単位)。多角形・折れ線は頂点の位置だけ。
fn params(shape: &Shape) -> &'static [(Param, &'static str, &'static str)] {
    match shape {
        Shape::Circle { .. } | Shape::Sphere { .. } => &[(Param::Radius, "半径", "m")],
        Shape::Rect { .. } => &[
            (Param::Width, "幅(東西)", "m"),
            (Param::Height, "高さ(南北)", "m"),
            (Param::Rotation, "回転", "°"),
        ],
        Shape::Sector { .. } => &[
            (Param::Radius, "半径", "m"),
            (Param::StartDeg, "開始方位", "°"),
            (Param::EndDeg, "終了方位", "°"),
        ],
        Shape::Cuboid { .. } => &[
            (Param::SizeEw, "幅(東西)", "m"),
            (Param::SizeNs, "奥行(南北)", "m"),
            (Param::SizeUp, "高さ", "m"),
            (Param::Heading, "方位", "°"),
        ],
        Shape::Cylinder { .. } | Shape::Cone { .. } => {
            &[(Param::Radius, "半径", "m"), (Param::Height, "高さ", "m")]
        }
        Shape::Polygon { .. } | Shape::Polyline { .. } => &[],
    }
}

fn param_get(shape: &Shape, param: Param) -> f64 {
    match (shape, param) {
        (
            Shape::Circle { radius, .. }
            | Shape::Sphere { radius, .. }
            | Shape::Sector { radius, .. }
            | Shape::Cylinder { radius, .. }
            | Shape::Cone { radius, .. },
            Param::Radius,
        ) => *radius,
        (Shape::Rect { width, .. }, Param::Width) => *width,
        (
            Shape::Rect { height, .. }
            | Shape::Cylinder { height, .. }
            | Shape::Cone { height, .. },
            Param::Height,
        ) => *height,
        (Shape::Rect { rotation_deg, .. }, Param::Rotation) => *rotation_deg,
        (Shape::Sector { start_deg, .. }, Param::StartDeg) => *start_deg,
        (Shape::Sector { end_deg, .. }, Param::EndDeg) => *end_deg,
        (Shape::Cuboid { size_m, .. }, Param::SizeEw) => size_m[0],
        (Shape::Cuboid { size_m, .. }, Param::SizeNs) => size_m[1],
        (Shape::Cuboid { size_m, .. }, Param::SizeUp) => size_m[2],
        (Shape::Cuboid { heading_deg, .. }, Param::Heading) => *heading_deg,
        _ => 0.0,
    }
}

/// 大きさは1m以上にする(0以下だと図形が描かれなくなり、見失うため)。角度はそのまま。
fn param_set(shape: &mut Shape, param: Param, v: f64) {
    let size = v.max(1.0);
    match (shape, param) {
        (
            Shape::Circle { radius, .. }
            | Shape::Sphere { radius, .. }
            | Shape::Sector { radius, .. }
            | Shape::Cylinder { radius, .. }
            | Shape::Cone { radius, .. },
            Param::Radius,
        ) => *radius = size,
        (Shape::Rect { width, .. }, Param::Width) => *width = size,
        (
            Shape::Rect { height, .. }
            | Shape::Cylinder { height, .. }
            | Shape::Cone { height, .. },
            Param::Height,
        ) => *height = size,
        (Shape::Rect { rotation_deg, .. }, Param::Rotation) => *rotation_deg = v,
        (Shape::Sector { start_deg, .. }, Param::StartDeg) => *start_deg = v,
        (Shape::Sector { end_deg, .. }, Param::EndDeg) => *end_deg = v,
        (Shape::Cuboid { size_m, .. }, Param::SizeEw) => size_m[0] = size,
        (Shape::Cuboid { size_m, .. }, Param::SizeNs) => size_m[1] = size,
        (Shape::Cuboid { size_m, .. }, Param::SizeUp) => size_m[2] = size,
        (Shape::Cuboid { heading_deg, .. }, Param::Heading) => *heading_deg = v,
        _ => {}
    }
}

/// 位置`index`の(緯度, 経度)。
fn position_get(shape: &Shape, index: usize) -> (f64, f64) {
    match shape.positions().get(index) {
        Some(Position::World {
            lat_deg, lon_deg, ..
        }) => (*lat_deg, *lon_deg),
        _ => (0.0, 0.0),
    }
}

/// 位置`index`の緯度・経度を書き換える(`None`の側は変えない)。
fn position_set(shape: &mut Shape, index: usize, lat: Option<f64>, lon: Option<f64>) {
    if let Some(Position::World {
        lat_deg, lon_deg, ..
    }) = shape.positions_mut().get_mut(index)
    {
        if let Some(lat) = lat {
            *lat_deg = lat.clamp(-90.0, 90.0);
        }
        if let Some(lon) = lon {
            *lon_deg = lon.clamp(-180.0, 180.0);
        }
    }
}

/// フォームを作り直す必要があるかの判定に使う、図形の種類の番号。
fn kind_tag(shape: &Shape) -> u8 {
    match shape {
        Shape::Circle { .. } => 0,
        Shape::Rect { .. } => 1,
        Shape::Polygon { .. } => 2,
        Shape::Sector { .. } => 3,
        Shape::Sphere { .. } => 4,
        Shape::Cuboid { .. } => 5,
        Shape::Cylinder { .. } => 6,
        Shape::Cone { .. } => 7,
        Shape::Polyline { .. } => 8,
    }
}

fn with_shape<R>(tool: DrawToolState, id: DrawingId, f: impl FnOnce(&Shape) -> R) -> Option<R> {
    tool.drawings.with(id, |d| f(&d.shape))
}

// ---------------------------------------------------------------------------------------------
// 選択中の図形の編集フォーム
// ---------------------------------------------------------------------------------------------

fn shape_form(tool: DrawToolState, id: DrawingId) -> AnyView {
    let Some(shape) = with_shape(tool, id, Shape::clone) else {
        return ().into_any();
    };
    let is_line = matches!(shape, Shape::Polyline { .. });
    let count = shape.positions().len();
    let multi = count > 1;

    let position_rows = (0..count)
        .map(|i| {
            let label = if multi { format!("点{}", i + 1) } else { "位置".to_string() };
            view! {
                <div class="drawing-row">
                    <span class="drawing-point-label">{label}</span>
                    {num_field(
                        "緯度",
                        "°",
                        move || with_shape(tool, id, |s| position_get(s, i).0).unwrap_or(0.0),
                        move |v| tool.update_shape(id, |d| position_set(&mut d.shape, i, Some(v), None)),
                    )}
                    {num_field(
                        "経度",
                        "°",
                        move || with_shape(tool, id, |s| position_get(s, i).1).unwrap_or(0.0),
                        move |v| tool.update_shape(id, |d| position_set(&mut d.shape, i, None, Some(v))),
                    )}
                </div>
            }
        })
        .collect_view();

    let param_fields = params(&shape)
        .iter()
        .map(|&(param, label, unit)| {
            num_field(
                label,
                unit,
                move || with_shape(tool, id, |s| param_get(s, param)).unwrap_or(0.0),
                move |v| tool.update_shape(id, |d| param_set(&mut d.shape, param, v)),
            )
        })
        .collect_view();

    let name = move || {
        tool.shapes.with(|list| {
            list.iter()
                .find(|u| u.id == id)
                .map(|u| u.name.clone())
                .unwrap_or_default()
        })
    };
    let altitude_get = move || {
        with_shape(tool, id, |s| match s.positions().first() {
            Some(Position::World { altitude, .. }) => *altitude,
            _ => Altitude::AboveGround(0.0),
        })
        .unwrap_or(Altitude::AboveGround(0.0))
    };
    let altitude_set = move |a: Altitude| {
        tool.update_shape(id, |d| {
            for p in d.shape.positions_mut() {
                if let Position::World { altitude, .. } = p {
                    *altitude = a;
                }
            }
        });
    };
    let style_get = move || tool.drawings.with(id, |d| d.style).unwrap_or_default();
    let style_set = move |style: Style| tool.update_shape(id, |d| d.style = style);

    view! {
        <div class="drawing-form">
            <label class="drawing-field drawing-name">
                <span>"名前"</span>
                <input
                    type="text"
                    prop:value=name
                    on:change=move |ev| tool.rename(id, event_target_value(&ev))
                />
            </label>
            {position_rows}
            <div class="drawing-row">{param_fields}</div>
            {altitude_fields(altitude_get, altitude_set)}
            {style_fields(style_get, style_set, !is_line)}
        </div>
    }
    .into_any()
}

// ---------------------------------------------------------------------------------------------
// パネル本体
// ---------------------------------------------------------------------------------------------

#[component]
pub fn DrawingEditor() -> impl IntoView {
    let tool = use_context::<DrawToolState>().expect("DrawToolState context not found");

    let tool_buttons = ToolKind::ALL
        .into_iter()
        .map(|kind| {
            view! {
                <button
                    class="drawing-tool-button"
                    class:active=move || tool.tool.get() == Some(kind)
                    on:click=move |_| tool.start(kind)
                >
                    {kind.label()}
                </button>
            }
        })
        .collect_view();

    // 一覧の行は「作った図形の一覧(追加・削除・改名)」でだけ作り直す。図形の中身の変化(編集や、作成中の
    // 仮の図形の更新)では作り直さない(入力中のフォーカスが外れるため)。表示/非表示は行の中で読む。
    let visible_of = move |id: DrawingId| tool.drawings.with(id, |d| d.visible).unwrap_or(true);
    // 一覧の行の右クリックメニュー(`ContextMenuState`が提供されていれば)。
    let context_menu = use_context::<ContextMenuState>();
    let rows = move |u: UserShape| {
        let id = u.id;
        let on_context_menu = move |ev: leptos::ev::MouseEvent| {
            let Some(menu) = context_menu else { return };
            ev.prevent_default();
            tool.select(Some(id));
            let visible = visible_of(id);
            menu.show(
                f64::from(ev.client_x()),
                f64::from(ev.client_y()),
                vec![
                    MenuItem::action("名前を変更", move || {
                        // 編集フォームは選択の変更のあとに作られるので、次のフレームで名前欄へフォーカスする。
                        tool.select(Some(id));
                        request_animation_frame(|| {
                            let input = web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| {
                                    d.query_selector(".drawing-form .drawing-name input")
                                        .ok()
                                        .flatten()
                                })
                                .and_then(|el| {
                                    wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(el).ok()
                                });
                            if let Some(input) = input {
                                let _ = input.focus();
                            }
                        });
                    }),
                    MenuItem::action("複製", move || tool.duplicate(id)),
                    MenuItem::action(
                        if visible {
                            "非表示にする"
                        } else {
                            "表示する"
                        },
                        move || {
                            tool.update_shape(id, |d| d.visible = !visible);
                        },
                    ),
                    MenuItem::separator(),
                    MenuItem::action("削除", move || tool.remove(id)),
                ],
            );
        };
        view! {
            <div
                class="drawing-row-item"
                class:selected=move || tool.selected.get() == Some(id)
                on:contextmenu=on_context_menu
            >
                <input
                    type="checkbox"
                    title="表示/非表示"
                    prop:checked=move || visible_of(id)
                    on:change=move |ev| {
                        let visible = event_target_checked(&ev);
                        tool.update_shape(id, |d| d.visible = visible);
                    }
                />
                <button
                    class="drawing-row-name"
                    title="選択して編集"
                    on:click=move |_| tool.select(if tool.selected.get_untracked() == Some(id) { None } else { Some(id) })
                >
                    {u.name}
                </button>
                <button class="los-delete-btn" title="この図形を削除" on:click=move |_| tool.remove(id)>
                    "削除"
                </button>
            </div>
        }
    };

    // 編集フォームは、選択が変わったときか、図形の種類・点の数が変わったときだけ作り直す。
    let form_key = Memo::new(move |_| {
        tool.selected.get().and_then(|id| {
            tool.drawings
                .with(id, |d| (id, kind_tag(&d.shape), d.shape.positions().len()))
        })
    });

    let on_remove_all = move |_| {
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message("作った図形をすべて削除しますか?")
                    .ok()
            })
            // 確認ダイアログを出せない環境では、確認なしに全削除しない。
            .unwrap_or(false);
        if confirmed {
            tool.remove_all();
        }
    };

    view! {
        <div class="drawing-editor">
            <p class="drawing-help">
                "図形を選んで、地図をクリックして置きます。数値は下の一覧から選んで編集できます。"
            </p>
            <div class="drawing-tool-grid">{tool_buttons}</div>

            <details class="drawing-section">
                <summary>"新しい図形の見た目・高度"</summary>
                {style_fields(move || tool.new_style.get(), move |s| tool.new_style.set(s), true)}
                {altitude_fields(move || tool.new_altitude.get(), move |a| tool.new_altitude.set(a))}
            </details>

            <div class="drawing-list-header">
                <span>{move || format!("作った図形({})", tool.shapes.with(|l| l.len()))}</span>
                <button class="los-delete-btn" on:click=on_remove_all disabled=move || tool.shapes.with(|l| l.is_empty())>
                    "すべて削除"
                </button>
            </div>
            <div class="drawing-list">
                <For each=move || tool.shapes.get() key=|u| (u.id, u.name.clone()) children=rows />
                {move || {
                    tool.shapes
                        .with(|l| l.is_empty())
                        .then(|| view! { <p class="placeholder">"まだ図形がありません"</p> })
                }}
            </div>
            {move || form_key.get().map(|(id, _, _)| shape_form(tool, id))}
        </div>
    }
}
