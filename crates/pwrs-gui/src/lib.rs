//! PWRS-GUI: windows, dialogs and grids for PowerShell, drawn by Rust
//! renderers in a helper process the module starts, or on Windows in the
//! module itself.

mod canvas;
mod choice;
mod dialogs;
mod entry;
mod gpu;
mod grid;
mod input;
mod inspect;
mod latency;
mod media;
mod options;
mod page;
mod progress;
mod session;
mod shared;
mod tree;
mod values;
mod widgets;
mod window;

use choice::ReadGuiChoice;
use dialogs::{GetGuiDialog, GuiDialogInfo, GuiMessageButtons, GuiMessageIcon, GuiMessageResult, ReadGuiFile, ReadGuiFolder, ReadGuiSavePath, ShowGuiMessage};
use entry::{GetGuiNativeEntry, NativeEntry};
use gpu::{
    CanvasView, GetGuiCanvasView, GetGuiGpuCamera, GpuBuffer, GpuCamera, GpuComputeResult, GpuDeviceEvent, GpuRenderPipeline, GpuSampler, GpuTexture, GuiAddressMode, GuiBlendFactor, GuiBlendMode,
    GuiBlendOperation, GuiBufferUsage, GuiCameraControls, GuiCanvasViewControls, GuiCompareFunction, GuiCullMode, GuiDepthFormat, GuiElementType, GuiFilterMode, GuiPrimitiveTopology, GuiShaderLanguage,
    GuiStencilOperation, GuiTextureFormat, ImportGuiGpuImage, InvokeGuiGpuCompute, NewGuiGpuBuffer, NewGuiGpuRenderPipeline, NewGuiGpuSampler, NewGuiGpuTexture, ReadGuiGpuBuffer, ReadGuiGpuTexture,
    RemoveGuiCanvasView, RemoveGuiGpuBuffer, RemoveGuiGpuCamera, RemoveGuiGpuTexture, ResetGuiGpuDevice, SaveGuiGpuSurface, SetGuiCanvasView, SetGuiGpuCamera, SetGuiGpuSurface, WriteGuiGpuBuffer,
    WriteGuiGpuTexture,
};
use grid::{GridOutputMode, OutGuiGridView};
use input::{ReadGuiCredential, ReadGuiInput};
use inspect::{GetGuiGpuAdapter, GetGuiSnapshot, GetGuiWindow, GpuAdapter, GuiEventKind, GuiSnapshot, GuiWindowInfo, InvokeGuiTestEvent};
use latency::{Clock, Latency, LatencyClick, LatencyRun, MeasureGuiLatency};
use media::{GetGuiMediaPlayer, GuiMediaPhase, InvokeGuiMedia, MediaPlayerState, SetGuiMediaPlayer};
use options::{GetGuiOption, GuiFont, GuiMode, GuiOption, GuiRenderer, GuiTheme, GuiWebView, SetGuiOption};
use page::{PageMessage, PageWindow, SendGuiMessage, ShowGuiWindow};
use progress::{GuiProgressWindow, SetGuiProgress, ShowGuiProgress};
use pwrs::prelude::*;
use widgets::{
    GuiAlign, GuiChartType, GuiImageFit, GuiOrientation, GuiSelection, GuiTextStyle, GuiWidget, NewGuiButton, NewGuiButtonBar, NewGuiCanvas, NewGuiChart, NewGuiCheckBox, NewGuiColorPicker, NewGuiColumn, NewGuiComboBox, NewGuiContextMenu,
    NewGuiDatePicker, NewGuiFilePicker, NewGuiGpuSurface, NewGuiGrid, NewGuiGroup, NewGuiImage, NewGuiListBox, NewGuiLogView, NewGuiMarkdown, NewGuiMediaPlayer, NewGuiMenu, NewGuiMenuBar, NewGuiMenuItem, NewGuiNumberBox,
    NewGuiStatusBar, NewGuiProgressBar, NewGuiRadioGroup, NewGuiRow, NewGuiScroll, NewGuiSeparator, NewGuiSlider, NewGuiSpacer, NewGuiSpinner, NewGuiSplit, NewGuiTab, NewGuiTable, NewGuiTabs, NewGuiText,
    NewGuiTextBox, NewGuiTreeView,
};
use window::{AddGuiItem, CloseGuiWindow, GuiHandlerOutput, GuiWidgetEvent, GuiWidgetEventData, RemoveGuiItem, SetGuiFocus, UpdateGuiWidget, WaitGuiEvent};

/// Ends the module's sessions once no runspace of the process has the
/// module imported, the last one having removed it or closed: their windows
/// close, each helper exits, and an in-process renderer's run ends. Every
/// runspace shares the sessions, so a runspace that removes the module or
/// closes while another has it imported leaves them running.
#[on_unload]
fn end_session() -> PsResult<()> {
    session::end();
    window::forget_all();
    Ok(())
}

pwrs::export_module! {
    name: "PWRS-GUI",
    cmdlets: [
        ReadGuiChoice,
        ReadGuiInput,
        ReadGuiCredential,
        OutGuiGridView,
        ShowGuiProgress,
        SetGuiProgress,
        ShowGuiMessage,
        ReadGuiFile,
        ReadGuiSavePath,
        ReadGuiFolder,
        GetGuiDialog,
        GetGuiWindow,
        InvokeGuiTestEvent,
        GetGuiSnapshot,
        GetGuiGpuAdapter,
        NewGuiGpuBuffer,
        ReadGuiGpuBuffer,
        WriteGuiGpuBuffer,
        RemoveGuiGpuBuffer,
        NewGuiGpuTexture,
        ImportGuiGpuImage,
        ReadGuiGpuTexture,
        WriteGuiGpuTexture,
        RemoveGuiGpuTexture,
        NewGuiGpuSampler,
        InvokeGuiGpuCompute,
        ResetGuiGpuDevice,
        NewGuiGpuRenderPipeline,
        SetGuiGpuSurface,
        SaveGuiGpuSurface,
        SetGuiGpuCamera,
        GetGuiGpuCamera,
        RemoveGuiGpuCamera,
        SetGuiCanvasView,
        GetGuiCanvasView,
        RemoveGuiCanvasView,
        SetGuiOption,
        GetGuiOption,
        MeasureGuiLatency,
        GetGuiNativeEntry,
        ShowGuiWindow,
        SendGuiMessage,
        NewGuiColumn,
        NewGuiRow,
        NewGuiGrid,
        NewGuiGroup,
        NewGuiTabs,
        NewGuiTab,
        NewGuiSplit,
        NewGuiScroll,
        NewGuiText,
        NewGuiImage,
        NewGuiMarkdown,
        NewGuiProgressBar,
        NewGuiSpinner,
        NewGuiLogView,
        NewGuiChart,
        NewGuiGpuSurface,
        NewGuiCanvas,
        NewGuiMediaPlayer,
        InvokeGuiMedia,
        SetGuiMediaPlayer,
        GetGuiMediaPlayer,
        NewGuiButton,
        NewGuiListBox,
        NewGuiTable,
        NewGuiTreeView,
        NewGuiCheckBox,
        NewGuiTextBox,
        NewGuiNumberBox,
        NewGuiSlider,
        NewGuiComboBox,
        NewGuiRadioGroup,
        NewGuiColorPicker,
        NewGuiDatePicker,
        NewGuiFilePicker,
        NewGuiMenuBar,
        NewGuiMenu,
        NewGuiMenuItem,
        NewGuiContextMenu,
        NewGuiStatusBar,
        NewGuiButtonBar,
        NewGuiSeparator,
        NewGuiSpacer,
        UpdateGuiWidget,
        SetGuiFocus,
        CloseGuiWindow,
        AddGuiItem,
        RemoveGuiItem,
        WaitGuiEvent,
    ],
    classes: [
        GuiOption,
        GuiWindowInfo,
        GuiSnapshot,
        GpuAdapter,
        GpuBuffer,
        GpuTexture,
        GpuSampler,
        GpuComputeResult,
        GpuDeviceEvent,
        GpuRenderPipeline,
        GpuCamera,
        CanvasView,
        MediaPlayerState,
        Clock,
        Latency,
        LatencyRun,
        LatencyClick,
        NativeEntry,
        PageWindow,
        PageMessage,
        GuiWidget,
        GuiWidgetEvent,
        GuiWidgetEventData,
        GuiProgressWindow,
        GuiDialogInfo,
    ],
    enums: [
        GuiMode,
        GuiRenderer,
        GuiTheme,
        GuiFont,
        GuiWebView,
        GridOutputMode,
        GuiEventKind,
        GuiTextStyle,
        GuiAlign,
        GuiImageFit,
        GuiHandlerOutput,
        GuiSelection,
        GuiOrientation,
        GuiChartType,
        GuiMessageIcon,
        GuiMessageButtons,
        GuiMessageResult,
        GuiElementType,
        GuiBufferUsage,
        GuiTextureFormat,
        GuiFilterMode,
        GuiAddressMode,
        GuiShaderLanguage,
        GuiPrimitiveTopology,
        GuiCullMode,
        GuiBlendMode,
        GuiBlendFactor,
        GuiBlendOperation,
        GuiDepthFormat,
        GuiCompareFunction,
        GuiStencilOperation,
        GuiCameraControls,
        GuiCanvasViewControls,
        GuiMediaPhase,
    ],
    on_unload: end_session,
}
