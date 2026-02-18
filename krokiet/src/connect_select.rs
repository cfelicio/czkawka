use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::common::connect_i32_into_u64;
use crate::connect_row_selection::checker::change_number_of_enabled_items;
use crate::connect_translation::translate_select_mode;
use crate::{ActiveTab, Callabler, GuiState, MainWindow, SelectMode, SelectModel, SingleMainListModel};
use regex::RegexBuilder;

type SelectionResult = (u64, u64, ModelRc<SingleMainListModel>);

// TODO optimize this, not sure if it is possible to not copy entire model to just select item
// https://github.com/slint-ui/slint/discussions/4595
pub(crate) fn connect_select(app: &MainWindow) {
    set_select_buttons(app);

    let a = app.as_weak();
    let a_select = a.clone();
    app.global::<Callabler>().on_select_items(move |select_mode| {
        let app = a_select.upgrade().expect("Failed to upgrade app :(");
        let active_tab = app.global::<GuiState>().get_active_tab();
        let current_model = active_tab.get_tool_model(&app);

        let (checked_items, unchecked_items, new_model) = match select_mode {
            SelectMode::SelectAll => select_all(&current_model),
            SelectMode::UnselectAll => deselect_all(&current_model),
            SelectMode::InvertSelection => invert_selection(&current_model),
            SelectMode::SelectTheBiggestSize => select_by_property(&current_model, active_tab, Property::Size, true),
            SelectMode::SelectTheSmallestSize => select_by_property(&current_model, active_tab, Property::Size, false),
            SelectMode::SelectTheBiggestResolution => select_by_property(&current_model, active_tab, Property::Resolution, true),
            SelectMode::SelectTheSmallestResolution => select_by_property(&current_model, active_tab, Property::Resolution, false),
            SelectMode::SelectNewest => select_by_property(&current_model, active_tab, Property::Date, true),
            SelectMode::SelectOldest => select_by_property(&current_model, active_tab, Property::Date, false),
            SelectMode::SelectShortestPath => select_by_property(&current_model, active_tab, Property::PathLength, false),
            SelectMode::SelectLongestPath => select_by_property(&current_model, active_tab, Property::PathLength, true),
        };
        active_tab.set_tool_model(&app, new_model);
        change_number_of_enabled_items(&app, active_tab, checked_items as i64 - unchecked_items as i64);
    });

    // Custom select callback (opened from Slint popup)
    app.global::<Callabler>().on_custom_select(
        move |select_matches, check_path, check_name, check_regex_path_name, case_sensitive, prevent_select_all_in_group, path_pattern, name_pattern, regex_pattern| {
            let app = a.upgrade().expect("Failed to upgrade app :(");
            let active_tab = app.global::<GuiState>().get_active_tab();
            let current_model = active_tab.get_tool_model(&app);

            let (checked_items, unchecked_items, new_model) = select_by_pattern(
                &current_model,
                active_tab,
                CustomSelectConfig {
                    select_matches,
                    check_path,
                    check_name,
                    check_regex_path_name,
                    case_sensitive,
                    prevent_select_all_in_group,
                    path_pattern: &path_pattern,
                    name_pattern: &name_pattern,
                    regex_pattern: &regex_pattern,
                },
            );
            active_tab.set_tool_model(&app, new_model);
            change_number_of_enabled_items(&app, active_tab, checked_items as i64 - unchecked_items as i64);
        },
    );

    app.global::<Callabler>().on_validate_regex(move |pattern| regex::Regex::new(&pattern).is_ok());
}

#[derive(Clone, Copy)]
struct CustomSelectConfig<'a> {
    select_matches: bool,
    check_path: bool,
    check_name: bool,
    check_regex_path_name: bool,
    case_sensitive: bool,
    prevent_select_all_in_group: bool,
    path_pattern: &'a str,
    name_pattern: &'a str,
    regex_pattern: &'a str,
}

// Select/unselect by wildcard or rust-regex path+name (used by popup)
fn select_by_pattern(model: &ModelRc<SingleMainListModel>, active_tab: ActiveTab, config: CustomSelectConfig<'_>) -> SelectionResult {
    let mut checked_items: u64 = 0;
    let mut unchecked_items: u64 = 0;
    let mut old_data = model.iter().collect::<Vec<_>>();

    if !(config.check_path || config.check_name || config.check_regex_path_name) {
        return (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data)));
    }

    let path_pattern = config.path_pattern.trim();
    let name_pattern = config.name_pattern.trim();
    let regex_pattern = config.regex_pattern.trim();

    let regex = if config.check_regex_path_name {
        if regex_pattern.is_empty() {
            return (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data)));
        }
        match RegexBuilder::new(regex_pattern).case_insensitive(!config.case_sensitive).build() {
            Ok(compiled) => Some(compiled),
            Err(_) => return (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data))),
        }
    } else {
        None
    };

    let wildcard_path = if config.check_path && !path_pattern.is_empty() {
        let normalized = if config.case_sensitive { path_pattern.to_string() } else { path_pattern.to_lowercase() };
        Some(czkawka_core::common::items::new_excluded_item(&normalized))
    } else {
        None
    };

    let wildcard_name = if config.check_name && !name_pattern.is_empty() {
        let normalized = if config.case_sensitive { name_pattern.to_string() } else { name_pattern.to_lowercase() };
        Some(czkawka_core::common::items::new_excluded_item(&normalized))
    } else {
        None
    };

    if regex.is_none() && wildcard_path.is_none() && wildcard_name.is_none() {
        return (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data)));
    }

    let mut matched_rows = vec![false; old_data.len()];
    for (idx, row) in old_data.iter().enumerate() {
        if row.header_row {
            continue;
        }

        let path = row.val_str.iter().nth(active_tab.get_str_path_idx()).map(|s| s.to_string()).unwrap_or_default();
        let name = row.val_str.iter().nth(active_tab.get_str_name_idx()).map(|s| s.to_string()).unwrap_or_default();

        let is_match = if let Some(re) = &regex {
            let full_name = if path.is_empty() {
                name.clone()
            } else if name.is_empty() {
                path.clone()
            } else {
                format!("{path}/{name}")
            };
            re.is_match(&full_name)
        } else {
            let mut wildcard_match = false;
            if let Some(path_wildcard) = &wildcard_path {
                let path_to_check = if config.case_sensitive { path.clone() } else { path.to_lowercase() };
                if czkawka_core::common::regex_check(path_wildcard, &path_to_check) {
                    wildcard_match = true;
                }
            }
            if let Some(name_wildcard) = &wildcard_name {
                let name_to_check = if config.case_sensitive { name } else { name.to_lowercase() };
                if czkawka_core::common::regex_check(name_wildcard, &name_to_check) {
                    wildcard_match = true;
                }
            }
            wildcard_match
        };

        matched_rows[idx] = is_match;
    }

    if config.select_matches {
        let mut grouped_ranges = Vec::new();
        if active_tab.get_is_header_mode() {
            grouped_ranges = collect_group_ranges(&old_data);
        }

        if config.prevent_select_all_in_group && active_tab.get_is_header_mode() {
            for (start_idx, end_idx) in grouped_ranges {
                let mut possible_to_select = Vec::new();
                let mut unchecked_items_in_group = 0usize;

                for row_idx in start_idx..end_idx {
                    let row = &old_data[row_idx];
                    if row.header_row {
                        continue;
                    }

                    if !row.checked {
                        unchecked_items_in_group += 1;
                        if matched_rows[row_idx] {
                            possible_to_select.push(row_idx);
                        }
                    }
                }

                if !possible_to_select.is_empty() && possible_to_select.len() == unchecked_items_in_group {
                    possible_to_select.pop();
                }

                for row_idx in possible_to_select {
                    if !old_data[row_idx].checked {
                        checked_items += 1;
                    }
                    old_data[row_idx].checked = true;
                }
            }
        } else {
            for (row_idx, row) in old_data.iter_mut().enumerate() {
                if row.header_row {
                    continue;
                }
                if matched_rows[row_idx] {
                    if !row.checked {
                        checked_items += 1;
                    }
                    row.checked = true;
                }
            }
        }
    } else {
        for (row_idx, row) in old_data.iter_mut().enumerate() {
            if row.header_row {
                continue;
            }
            if matched_rows[row_idx] {
                if row.checked {
                    unchecked_items += 1;
                }
                row.checked = false;
            }
        }
    }

    (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data)))
}

fn collect_group_ranges(old_data: &[SingleMainListModel]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut group_start = 0usize;

    for (idx, row) in old_data.iter().enumerate() {
        if row.header_row {
            if group_start < idx {
                ranges.push((group_start, idx));
            }
            group_start = idx + 1;
        }
    }

    if group_start < old_data.len() {
        ranges.push((group_start, old_data.len()));
    }

    if ranges.is_empty() && !old_data.is_empty() {
        ranges.push((0, old_data.len()));
    }

    ranges
}

#[derive(Clone, Copy)]
enum Property {
    Size,
    Date,
    PathLength,
    Resolution,
}

pub(crate) fn set_select_buttons(app: &MainWindow) {
    let active_tab = app.global::<GuiState>().get_active_tab();
    let mut base_buttons = vec![SelectMode::SelectAll, SelectMode::UnselectAll, SelectMode::InvertSelection];

    let additional_buttons = match active_tab {
        ActiveTab::DuplicateFiles | ActiveTab::SimilarVideos | ActiveTab::SimilarMusic => vec![
            SelectMode::SelectOldest,
            SelectMode::SelectNewest,
            SelectMode::SelectTheSmallestSize,
            SelectMode::SelectTheBiggestSize,
            SelectMode::SelectShortestPath,
            SelectMode::SelectLongestPath,
        ],
        ActiveTab::SimilarImages => vec![
            SelectMode::SelectOldest,
            SelectMode::SelectNewest,
            SelectMode::SelectTheSmallestSize,
            SelectMode::SelectTheBiggestSize,
            SelectMode::SelectTheSmallestResolution,
            SelectMode::SelectTheBiggestResolution,
            SelectMode::SelectShortestPath,
            SelectMode::SelectLongestPath,
        ],
        ActiveTab::EmptyFolders
        | ActiveTab::BigFiles
        | ActiveTab::EmptyFiles
        | ActiveTab::TemporaryFiles
        | ActiveTab::InvalidSymlinks
        | ActiveTab::BrokenFiles
        | ActiveTab::BadExtensions
        | ActiveTab::BadNames
        | ActiveTab::ExifRemover
        | ActiveTab::VideoOptimizer
        | ActiveTab::Settings
        | ActiveTab::About => Vec::new(), // Not available in settings and about, so may be set any value here
    };

    base_buttons.extend(additional_buttons);
    base_buttons.reverse();

    let new_select_model = base_buttons
        .into_iter()
        .map(|e| SelectModel {
            name: translate_select_mode(e),
            data: e,
        })
        .collect::<Vec<_>>();

    app.global::<GuiState>().set_select_results_list(ModelRc::new(VecModel::from(new_select_model)));
}

fn extract_comparable_field(model: &SingleMainListModel, property: Property, active_tab: ActiveTab) -> u64 {
    let mut val_ints = model.val_int.iter();
    let mut val_strs = model.val_str.iter();
    match property {
        Property::Size => {
            let high = val_ints.nth(active_tab.get_int_size_idx()).expect("can find file size property");
            let low = val_ints.next().expect("can find file size property");
            connect_i32_into_u64(high, low)
        }
        Property::Date => {
            let high = val_ints.nth(active_tab.get_int_modification_date_idx()).expect("can find file last modified property");
            let low = val_ints.next().expect("can find file last modified property");
            connect_i32_into_u64(high, low)
        }
        Property::PathLength => val_strs.nth(active_tab.get_str_path_idx()).expect("can find file path property").len() as u64,
        Property::Resolution => val_ints.nth(active_tab.get_int_pixel_count_idx()).expect("can find pixel count proerty") as u64,
    }
}

fn select_by_property(model: &ModelRc<SingleMainListModel>, active_tab: ActiveTab, property: Property, increasing_order: bool) -> SelectionResult {
    let mut checked_items = 0;

    let is_header_mode = active_tab.get_is_header_mode();
    assert!(is_header_mode); // non header modes not really have reason to use this function

    let mut old_data = model.iter().collect::<Vec<_>>();
    let headers_idx = find_header_idx_and_deselect_all(&mut old_data);
    if increasing_order {
        for i in 0..(headers_idx.len() - 1) {
            let group_start = headers_idx[i] + 1;
            let group_end = headers_idx[i + 1];
            if group_start >= group_end {
                continue;
            }

            let mut max_item_idx = group_start;
            let mut max_item = extract_comparable_field(&old_data[max_item_idx], property, active_tab);

            #[expect(clippy::needless_range_loop)]
            for j in (group_start + 1)..group_end {
                let item = extract_comparable_field(&old_data[j], property, active_tab);
                if item > max_item {
                    max_item = item;
                    max_item_idx = j;
                }
            }
            if !old_data[max_item_idx].checked {
                checked_items += 1;
            }
            old_data[max_item_idx].checked = true;
        }
    } else {
        for i in 0..(headers_idx.len() - 1) {
            let group_start = headers_idx[i] + 1;
            let group_end = headers_idx[i + 1];
            if group_start >= group_end {
                continue;
            }

            let mut min_item_idx = group_start;
            let mut min_item = extract_comparable_field(&old_data[min_item_idx], property, active_tab);

            #[expect(clippy::needless_range_loop)]
            for j in (group_start + 1)..group_end {
                let item = extract_comparable_field(&old_data[j], property, active_tab);
                if item < min_item {
                    min_item = item;
                    min_item_idx = j;
                }
            }
            if !old_data[min_item_idx].checked {
                checked_items += 1;
            }
            old_data[min_item_idx].checked = true;
        }
    }

    (checked_items, 0, ModelRc::new(VecModel::from(old_data)))
}

fn select_all(model: &ModelRc<SingleMainListModel>) -> SelectionResult {
    let mut checked_items = 0;
    let mut old_data = model.iter().collect::<Vec<_>>();
    for x in &mut old_data {
        if !x.header_row {
            if !x.checked {
                checked_items += 1;
            }
            x.checked = true;
        }
    }
    (checked_items, 0, ModelRc::new(VecModel::from(old_data)))
}

fn deselect_all(model: &ModelRc<SingleMainListModel>) -> SelectionResult {
    let mut unchecked_items = 0;
    let mut old_data = model.iter().collect::<Vec<_>>();
    for x in &mut old_data {
        if x.checked {
            unchecked_items += 1;
        }
        x.checked = false;
    }
    (0, unchecked_items, ModelRc::new(VecModel::from(old_data)))
}

fn invert_selection(model: &ModelRc<SingleMainListModel>) -> SelectionResult {
    let mut checked_items = 0;
    let mut unchecked_items = 0;
    let mut old_data = model.iter().collect::<Vec<_>>();
    for x in &mut old_data {
        if !x.header_row {
            if x.checked {
                unchecked_items += 1;
            } else {
                checked_items += 1;
            }

            x.checked = !x.checked;
        }
    }
    (checked_items, unchecked_items, ModelRc::new(VecModel::from(old_data)))
}

fn find_header_idx_and_deselect_all(old_data: &mut [SingleMainListModel]) -> Vec<usize> {
    let mut header_idx = old_data
        .iter()
        .enumerate()
        .filter_map(|(idx, m)| if m.header_row { Some(idx) } else { None })
        .collect::<Vec<_>>();
    header_idx.push(old_data.len());

    for x in old_data.iter_mut() {
        if !x.header_row {
            x.checked = false;
        }
    }
    header_idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::create_model_from_model_vec;
    use crate::common::{MAX_INT_DATA_SIMILAR_IMAGES, MAX_STR_DATA_SIMILAR_IMAGES, split_u64_into_i32s};
    use crate::test_common::{get_main_list_model, get_model_vec};

    fn create_similar_images_row(path: &str, name: &str, checked: bool, header_row: bool) -> SingleMainListModel {
        let mut row = get_main_list_model();
        row.checked = checked;
        row.header_row = header_row;
        let mut val_str_data = vec![slint::SharedString::default(); MAX_STR_DATA_SIMILAR_IMAGES];
        val_str_data[crate::ActiveTab::SimilarImages.get_str_name_idx()] = name.into();
        val_str_data[crate::ActiveTab::SimilarImages.get_str_path_idx()] = path.into();
        row.val_str = ModelRc::new(VecModel::from(val_str_data));
        row
    }

    fn create_similar_images_row_with_metrics(size: i32, pixel_count: i32, checked: bool, header_row: bool) -> SingleMainListModel {
        let mut row = create_similar_images_row("", "", checked, header_row);
        let mut int_data = vec![0; MAX_INT_DATA_SIMILAR_IMAGES];
        let size_idx = crate::ActiveTab::SimilarImages.get_int_size_idx();
        int_data[size_idx] = 0;
        int_data[size_idx + 1] = size;
        int_data[crate::ActiveTab::SimilarImages.get_int_pixel_count_idx()] = pixel_count;
        row.val_int = ModelRc::new(VecModel::from(int_data));
        row
    }

    fn create_similar_images_row_with_all_metrics(path: &str, name: &str, size: i32, pixel_count: i32, date: u64, checked: bool, header_row: bool) -> SingleMainListModel {
        let mut row = create_similar_images_row(path, name, checked, header_row);
        let mut int_data = vec![0; MAX_INT_DATA_SIMILAR_IMAGES];
        let size_idx = crate::ActiveTab::SimilarImages.get_int_size_idx();
        int_data[size_idx] = 0;
        int_data[size_idx + 1] = size;
        int_data[crate::ActiveTab::SimilarImages.get_int_pixel_count_idx()] = pixel_count;

        let (date_part1, date_part2) = split_u64_into_i32s(date);
        let date_idx = crate::ActiveTab::SimilarImages.get_int_modification_date_idx();
        int_data[date_idx] = date_part1;
        int_data[date_idx + 1] = date_part2;

        row.val_int = ModelRc::new(VecModel::from(int_data));
        row
    }

    #[test]
    fn find_header_idx_returns_correct_indices_for_headers() {
        let mut model = get_model_vec(5);
        model[1].header_row = true;
        model[3].header_row = true;

        let header_indices = find_header_idx_and_deselect_all(&mut model);

        assert_eq!(header_indices, vec![1, 3, 5]);
    }

    #[test]
    fn find_header_idx_marks_all_non_header_rows_as_unchecked() {
        let mut model = get_model_vec(5);
        for row in &mut model {
            row.checked = true;
        }
        model[1].header_row = true;

        find_header_idx_and_deselect_all(&mut model);

        assert!(!model[0].checked);
        assert!(model[1].checked); // header row
        assert!(!model[2].checked);
        assert!(!model[3].checked);
        assert!(!model[4].checked);
    }

    #[test]
    fn select_all_marks_all_non_header_rows_as_checked() {
        let mut model = get_model_vec(5);
        model[1].header_row = true;
        let model = create_model_from_model_vec(&model);

        let (checked_items, unchecked_items, new_model) = select_all(&model);

        assert_eq!(checked_items, 4);
        assert_eq!(unchecked_items, 0);
        assert!(new_model.row_data(0).unwrap().checked);
        assert!(!new_model.row_data(1).unwrap().checked); // header row
        assert!(new_model.row_data(2).unwrap().checked);
        assert!(new_model.row_data(3).unwrap().checked);
        assert!(new_model.row_data(4).unwrap().checked);
    }

    #[test]
    fn deselect_all_unmarks_all_rows_as_checked() {
        let mut model = get_model_vec(5);
        for row in &mut model {
            row.checked = true;
        }
        let model = create_model_from_model_vec(&model);

        let (checked_items, unchecked_items, new_model) = deselect_all(&model);

        assert_eq!(checked_items, 0);
        assert_eq!(unchecked_items, 5);
        assert!(!new_model.row_data(0).unwrap().checked);
        assert!(!new_model.row_data(1).unwrap().checked);
        assert!(!new_model.row_data(2).unwrap().checked);
        assert!(!new_model.row_data(3).unwrap().checked);
        assert!(!new_model.row_data(4).unwrap().checked);
    }

    #[test]
    fn invert_selection_toggles_checked_state_for_non_header_rows() {
        let mut model = get_model_vec(5);
        model[0].checked = true;
        model[1].header_row = true;
        model[2].checked = false;
        let model = create_model_from_model_vec(&model);

        let (checked_items, unchecked_items, new_model) = invert_selection(&model);

        assert_eq!(checked_items, 3);
        assert_eq!(unchecked_items, 1);
        assert!(!new_model.row_data(0).unwrap().checked);
        assert!(!new_model.row_data(1).unwrap().checked); // header row
        assert!(new_model.row_data(2).unwrap().checked);
        assert!(new_model.row_data(3).unwrap().checked);
        assert!(new_model.row_data(4).unwrap().checked);
    }

    #[test]
    fn test_select_by_pattern_name_regex() {
        let model_data = vec![
            create_similar_images_row("/a", "nomatch.jpg", false, false),
            create_similar_images_row("/b", "file_match_name.jpg", false, false),
            create_similar_images_row("/c", "another.jpg", true, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (checked_items, unchecked_items, new_model) = select_by_pattern(
            &model,
            crate::ActiveTab::SimilarImages,
            CustomSelectConfig {
                select_matches: true,
                check_path: false,
                check_name: true,
                check_regex_path_name: false,
                case_sensitive: false,
                prevent_select_all_in_group: false,
                path_pattern: "",
                name_pattern: "*match*",
                regex_pattern: "",
            },
        );
        assert_eq!(checked_items, 1);
        assert_eq!(unchecked_items, 0);
        assert!(!new_model.row_data(0).unwrap().checked);
        assert!(new_model.row_data(1).unwrap().checked);
        assert!(new_model.row_data(2).unwrap().checked); // preserved previous selection
    }

    #[test]
    fn test_select_by_pattern_regex_path_and_name_unselect() {
        let model_data = vec![
            create_similar_images_row("/a", "one.jpg", true, false),
            create_similar_images_row("/b/subdir", "two.jpg", true, false),
            create_similar_images_row("/c", "three.jpg", true, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (checked_items, unchecked_items, new_model) = select_by_pattern(
            &model,
            crate::ActiveTab::SimilarImages,
            CustomSelectConfig {
                select_matches: false,
                check_path: false,
                check_name: false,
                check_regex_path_name: true,
                case_sensitive: false,
                prevent_select_all_in_group: false,
                path_pattern: "",
                name_pattern: "",
                regex_pattern: "subdir/.+",
            },
        );

        assert_eq!(checked_items, 0);
        assert_eq!(unchecked_items, 1);
        assert!(new_model.row_data(0).unwrap().checked);
        assert!(!new_model.row_data(1).unwrap().checked);
        assert!(new_model.row_data(2).unwrap().checked);
    }

    #[test]
    fn test_select_by_pattern_prevent_full_group_selection() {
        let model_data = vec![
            create_similar_images_row("", "", false, true), // header
            create_similar_images_row("/a", "one.jpg", false, false),
            create_similar_images_row("/b", "two.jpg", false, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (checked_items, unchecked_items, new_model) = select_by_pattern(
            &model,
            crate::ActiveTab::SimilarImages,
            CustomSelectConfig {
                select_matches: true,
                check_path: false,
                check_name: true,
                check_regex_path_name: false,
                case_sensitive: false,
                prevent_select_all_in_group: true,
                path_pattern: "",
                name_pattern: "*.jpg",
                regex_pattern: "",
            },
        );

        assert_eq!(checked_items, 1);
        assert_eq!(unchecked_items, 0);
        let selected_count = [new_model.row_data(1).unwrap().checked, new_model.row_data(2).unwrap().checked]
            .iter()
            .filter(|e| **e)
            .count();
        assert_eq!(selected_count, 1);
    }

    #[test]
    fn test_select_by_property_biggest_and_smallest_resolution() {
        let model_data = vec![
            create_similar_images_row_with_metrics(10, 100, false, true), // header
            create_similar_images_row_with_metrics(10, 100, false, false),
            create_similar_images_row_with_metrics(10, 300, false, false),
            create_similar_images_row_with_metrics(10, 200, false, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (_checked_biggest, _unchecked_biggest, biggest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::Resolution, true);
        assert!(!biggest_model.row_data(1).unwrap().checked);
        assert!(biggest_model.row_data(2).unwrap().checked);
        assert!(!biggest_model.row_data(3).unwrap().checked);

        let (_checked_smallest, _unchecked_smallest, smallest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::Resolution, false);
        assert!(smallest_model.row_data(1).unwrap().checked);
        assert!(!smallest_model.row_data(2).unwrap().checked);
        assert!(!smallest_model.row_data(3).unwrap().checked);
    }

    #[test]
    fn test_select_by_property_handles_empty_groups_without_corrupting_selection() {
        let model_data = vec![
            create_similar_images_row_with_metrics(0, 0, false, true), // header
            create_similar_images_row_with_metrics(0, 0, false, true), // header -> empty group between headers
            create_similar_images_row_with_metrics(10, 100, false, false),
            create_similar_images_row_with_metrics(20, 200, false, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (checked_items, _unchecked_items, new_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::Size, true);

        assert_eq!(checked_items, 1);
        assert!(!new_model.row_data(0).unwrap().checked);
        assert!(!new_model.row_data(1).unwrap().checked);
        assert!(!new_model.row_data(2).unwrap().checked);
        assert!(new_model.row_data(3).unwrap().checked);
    }

    #[test]
    fn test_select_by_property_newest_and_oldest() {
        let model_data = vec![
            create_similar_images_row_with_all_metrics("/a", "a.jpg", 10, 100, 10, false, true), // header
            create_similar_images_row_with_all_metrics("/a", "a.jpg", 10, 100, 100, false, false),
            create_similar_images_row_with_all_metrics("/b", "b.jpg", 10, 100, 300, false, false),
            create_similar_images_row_with_all_metrics("/c", "c.jpg", 10, 100, 200, false, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (_checked_newest, _unchecked_newest, newest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::Date, true);
        assert!(!newest_model.row_data(1).unwrap().checked);
        assert!(newest_model.row_data(2).unwrap().checked);
        assert!(!newest_model.row_data(3).unwrap().checked);

        let (_checked_oldest, _unchecked_oldest, oldest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::Date, false);
        assert!(oldest_model.row_data(1).unwrap().checked);
        assert!(!oldest_model.row_data(2).unwrap().checked);
        assert!(!oldest_model.row_data(3).unwrap().checked);
    }

    #[test]
    fn test_select_by_property_longest_and_shortest_path() {
        let model_data = vec![
            create_similar_images_row_with_all_metrics("", "", 10, 100, 0, false, true), // header
            create_similar_images_row_with_all_metrics("/a", "a.jpg", 10, 100, 0, false, false),
            create_similar_images_row_with_all_metrics("/very/long/path/for/testing", "b.jpg", 10, 100, 0, false, false),
            create_similar_images_row_with_all_metrics("/mid/path", "c.jpg", 10, 100, 0, false, false),
        ];
        let model = create_model_from_model_vec(&model_data);

        let (_checked_longest, _unchecked_longest, longest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::PathLength, true);
        assert!(!longest_model.row_data(1).unwrap().checked);
        assert!(longest_model.row_data(2).unwrap().checked);
        assert!(!longest_model.row_data(3).unwrap().checked);

        let (_checked_shortest, _unchecked_shortest, shortest_model) = select_by_property(&model, crate::ActiveTab::SimilarImages, Property::PathLength, false);
        assert!(shortest_model.row_data(1).unwrap().checked);
        assert!(!shortest_model.row_data(2).unwrap().checked);
        assert!(!shortest_model.row_data(3).unwrap().checked);
    }
}
