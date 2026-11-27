#import <AppKit/AppKit.h>
#import <Security/AuthSession.h>
#include <unistd.h>

@interface HostlaneMenu : NSObject
@property(nonatomic, strong) NSURL *desktop;
@end

@implementation HostlaneMenu
- (void)openDesktop:(id)sender {
    [NSWorkspace.sharedWorkspace openApplicationAtURL:self.desktop
                                        configuration:[NSWorkspaceOpenConfiguration configuration]
                                    completionHandler:nil];
}
@end

static NSURL *bundledDesktop(void) {
    NSURL *cli = [[NSURL fileURLWithPath:NSProcessInfo.processInfo.arguments.firstObject] URLByResolvingSymlinksInPath];
    NSURL *bin = cli.URLByDeletingLastPathComponent;
    NSURL *resources = bin.URLByDeletingLastPathComponent;
    NSURL *contents = resources.URLByDeletingLastPathComponent;
    NSURL *app = contents.URLByDeletingLastPathComponent;
    if (![bin.lastPathComponent isEqualToString:@"bin"] ||
        ![resources.lastPathComponent isEqualToString:@"Resources"] ||
        ![contents.lastPathComponent isEqualToString:@"Contents"] ||
        ![app.pathExtension isEqualToString:@"app"] ||
        ![[NSBundle bundleWithURL:app].bundleIdentifier isEqualToString:@"com.hostlane.app"]) return nil;
    return app;
}

void hostlane_menu_bar(const unsigned char *icon, size_t length) {
    @autoreleasepool {
        SessionAttributeBits attributes = 0;
        if (SessionGetInfo(callerSecuritySession, NULL, &attributes) != noErr || !(attributes & sessionHasGraphicAccess)) return;
        NSApplication *application = [NSApplication sharedApplication];
        [application setActivationPolicy:NSApplicationActivationPolicyAccessory];
        NSStatusItem * __attribute__((objc_precise_lifetime)) item = [NSStatusBar.systemStatusBar statusItemWithLength:NSSquareStatusItemLength];
        NSImage *image = [[NSImage alloc] initWithData:[NSData dataWithBytes:icon length:length]];
        image.size = NSMakeSize(20, 20);
        item.button.image = image;
        BOOL chinese = [NSLocale.preferredLanguages.firstObject hasPrefix:@"zh"];
        NSString *status = chinese ? @"Hostlane 守护进程正在运行" : @"Hostlane daemon is running";
        item.button.toolTip = status;
        [item.button setAccessibilityLabel:status];
        HostlaneMenu * __attribute__((objc_precise_lifetime)) target = [HostlaneMenu new];
        target.desktop = bundledDesktop();
        NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Hostlane"];
        [menu addItemWithTitle:status action:NULL keyEquivalent:@""];
        if (target.desktop) {
            [menu addItem:NSMenuItem.separatorItem];
            NSMenuItem *open = [menu addItemWithTitle:chinese ? @"打开 Hostlane" : @"Open Hostlane" action:@selector(openDesktop:) keyEquivalent:@""];
            open.target = target;
        }
        item.menu = menu;
        dispatch_async(dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{
            char buffer[32];
            ssize_t count;
            do { count = read(STDIN_FILENO, buffer, sizeof buffer); } while (count > 0 || (count < 0 && errno == EINTR));
            dispatch_async(dispatch_get_main_queue(), ^{ [application terminate:nil]; });
        });
        [application run];
    }
}
