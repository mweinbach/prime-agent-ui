#import <AppKit/AppKit.h>
#import <Sparkle/Sparkle.h>

// Rune/winit owns the application delegate and installs its native menu as the
// event loop starts. Activation also fires when the menu follows the launch notification.
@interface PrimeAgentUpdates : NSObject
@end

@implementation PrimeAgentUpdates
static SPUStandardUpdaterController *updater;

+ (void)load {
    NSNotificationCenter *notifications = NSNotificationCenter.defaultCenter;
    [notifications addObserver:self selector:@selector(applicationReady:)
        name:NSApplicationDidFinishLaunchingNotification object:nil];
    [notifications addObserver:self selector:@selector(applicationReady:)
        name:NSApplicationDidBecomeActiveNotification object:nil];
}

+ (void)applicationReady:(NSNotification *)notification {
    (void)notification;
    dispatch_async(dispatch_get_main_queue(), ^{
        NSMenu *menu = NSApp.mainMenu.itemArray.firstObject.submenu;
        if (!menu || ![NSBundle.mainBundle objectForInfoDictionaryKey:@"SUFeedURL"]) {
            return;
        }
        if (!updater) {
            updater = [[SPUStandardUpdaterController alloc] initWithStartingUpdater:YES
                updaterDelegate:nil userDriverDelegate:nil];
        }
        SEL checkForUpdates = @selector(checkForUpdates:);
        if ([menu indexOfItemWithTarget:updater andAction:checkForUpdates] >= 0) {
            return;
        }
        NSMenuItem *check = [[NSMenuItem alloc] initWithTitle:@"Check for Updates…"
            action:checkForUpdates keyEquivalent:@""];
        check.target = updater;
        [menu insertItem:check atIndex:MIN(1, menu.numberOfItems)];
    });
}
@end
