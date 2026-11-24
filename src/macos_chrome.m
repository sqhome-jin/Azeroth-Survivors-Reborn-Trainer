#import <AppKit/AppKit.h>

static NSString *const LanguagePreference = @"TrainerLanguage";
static NSString *const AppearancePreference = @"TrainerAppearance";

static BOOL UsesChinese(void) {
    NSString *language = [NSUserDefaults.standardUserDefaults stringForKey:LanguagePreference];
    return [(language ?: NSLocale.preferredLanguages.firstObject) hasPrefix:@"zh"];
}

static void ApplyAppearance(void) {
    NSInteger choice = [NSUserDefaults.standardUserDefaults integerForKey:AppearancePreference];
    NSApp.appearance = choice == 1 ? [NSAppearance appearanceNamed:NSAppearanceNameAqua] :
                       choice == 2 ? [NSAppearance appearanceNamed:NSAppearanceNameDarkAqua] : nil;
}

@interface TrainerTitlebar : NSTitlebarAccessoryViewController
@property(nonatomic, strong) NSPopUpButton *language;
@property(nonatomic, strong) NSPopUpButton *appearance;
@end

@implementation TrainerTitlebar
- (void)refreshLabels {
    BOOL chinese = UsesChinese();
    [self.language setAccessibilityLabel:chinese ? @"语言" : @"Language"];
    [self.language selectItemAtIndex:chinese ? 1 : 0];
    [self.appearance setAccessibilityLabel:chinese ? @"外观" : @"Appearance"];
    [self.appearance removeAllItems];
    [self.appearance addItemsWithTitles:chinese ? @[@"跟随系统", @"浅色", @"深色"] :
                                                      @[@"System", @"Light", @"Dark"]];
    NSInteger choice = [NSUserDefaults.standardUserDefaults integerForKey:AppearancePreference];
    [self.appearance selectItemAtIndex:choice >= 0 && choice <= 2 ? choice : 0];
    self.view.window.title = chinese ? @"Reborn 修改器" : @"Azeroth Survivor Reborn Trainer";
}

- (void)changeLanguage:(NSPopUpButton *)sender {
    [NSUserDefaults.standardUserDefaults setObject:sender.indexOfSelectedItem == 1 ? @"zh-Hans" : @"en"
                                           forKey:LanguagePreference];
    [self refreshLabels];
}

- (void)changeAppearance:(NSPopUpButton *)sender {
    [NSUserDefaults.standardUserDefaults setInteger:sender.indexOfSelectedItem forKey:AppearancePreference];
    ApplyAppearance();
}
@end

int trainer_install_macos_chrome(void *rawView) {
    NSView *view = (__bridge NSView *)rawView;
    NSWindow *window = view.window;
    if (!window) {
        return 0;
    }
    ApplyAppearance();
    TrainerTitlebar *accessory = [TrainerTitlebar new];
    accessory.layoutAttribute = NSLayoutAttributeRight;

    NSRect bounds = NSMakeRect(0, 0, 220, 30);
    NSView *container = [[NSView alloc] initWithFrame:bounds];
    NSView *controls = [[NSView alloc] initWithFrame:bounds];
    accessory.language = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(8, 3, 93, 24) pullsDown:NO];
    accessory.appearance = [[NSPopUpButton alloc] initWithFrame:NSMakeRect(101, 3, 99, 24) pullsDown:NO];
    [accessory.language addItemsWithTitles:@[@"English", @"简体中文"]];
    accessory.language.target = accessory;
    accessory.language.action = @selector(changeLanguage:);
    accessory.appearance.target = accessory;
    accessory.appearance.action = @selector(changeAppearance:);
    [controls addSubview:accessory.language];
    [controls addSubview:accessory.appearance];

#if __MAC_OS_X_VERSION_MAX_ALLOWED >= 260000
    if (@available(macOS 26.0, *)) {
        Class glassClass = NSClassFromString(@"NSGlassEffectView");
        NSGlassEffectView *glass = [[glassClass alloc] initWithFrame:NSInsetRect(bounds, 5, 2)];
        if (glass) {
            glass.cornerRadius = 13;
            accessory.language.bordered = NO;
            accessory.appearance.bordered = NO;
            glass.contentView = controls;
            [container addSubview:glass];
        }
    }
#endif
    if (container.subviews.count == 0) {
        [container addSubview:controls];
    }
    accessory.view = container;
    [window addTitlebarAccessoryViewController:accessory];
    [accessory refreshLabels];
    return 1;
}

int trainer_macos_preferences(void) {
    NSAppearanceName appearance = [NSApp.effectiveAppearance bestMatchFromAppearancesWithNames:
                                    @[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]];
    return (UsesChinese() ? 1 : 0) | ([appearance isEqualToString:NSAppearanceNameDarkAqua] ? 2 : 0);
}
