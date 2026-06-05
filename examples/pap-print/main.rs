use clap::Parser;
use tailtalk::{
    TalkStack,
    atp::AtpAddress,
};
use tailtalk_packets::nbp::EntityName;

#[derive(Parser, Debug)]
#[command(about = "Print a PostScript file to a PAP-capable AppleTalk printer")]
struct Args {
    /// Network interface to bind to
    #[arg(short, long)]
    interface: Option<String>,

    /// TashTalk serial port path (LocalTalk)
    #[arg(short, long)]
    tashtalk: Option<String>,

    /// Printer entity name to look up, e.g. "LaserWriter 4/600:LaserWriter@*"
    #[arg(short, long, default_value = "=:LaserWriter@*")]
    printer: String,

    /// File to print (omit for a built-in test page)
    #[arg(short, long)]
    file: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    let args = Args::parse();

    if args.interface.is_none() && args.tashtalk.is_none() {
        anyhow::bail!("at least one of --interface or --tashtalk is required");
    }

    let entity: EntityName = args
        .printer
        .as_str()
        .try_into()
        .map_err(|e| anyhow::anyhow!("Invalid printer name: {}", e))?;

    let mut builder = TalkStack::builder();
    if let Some(ref intf) = args.interface {
        builder = builder.ethernet(intf);
    }
    if let Some(ref tty) = args.tashtalk {
        builder = builder.localtalk(tty);
    }
    let stack = builder.build().await.expect("failed to build AppleTalk stack");

    // Locate the printer via NBP
    println!("Looking up printer '{}'...", entity);
    let tuples = stack.nbp.lookup(entity).await?;
    let printer = tuples
        .first()
        .ok_or_else(|| anyhow::anyhow!("Printer not found on network"))?;

    println!(
        "Found printer {} at {}.{} socket {}",
        printer.entity_name, printer.network_number, printer.node_id, printer.socket_number
    );

    let printer_addr = AtpAddress {
        network_number: printer.network_number,
        node_number: printer.node_id,
        socket_number: printer.socket_number,
    };

    // Query status
    println!("Querying printer status...");
    match stack.pap_status(printer_addr).await {
        Ok(status) => println!("Printer status: '{}'", status),
        Err(e) => println!("Could not get status: {}", e),
    }

    // Prepare data
    let data = if let Some(path) = &args.file {
        println!("Reading file '{}'...", path);
        std::fs::read(path)?
    } else if printer.entity_name.object.contains("LaserWriter") {
        println!("Using built-in test page...");

        b"%!PS-Adobe-2.0
%%Title: TailTalk Test Page
%%Creator: TailTalk pap-print
%%EndComments
/Courier findfont 15 scalefont setfont
72 720 moveto
(TailTalk PAP Test) show
showpage
"
        .to_vec()
    } else {
        println!("Generating test page...");
                b"Hello, ImageWriter!
"
                .to_vec()
    };

    println!("Connecting to printer ({} bytes to send)...", data.len());
    let mut client = stack.pap_client().await;
    client.connect(printer_addr).await?;

    println!("Printing...");
    client.print(&data).await?;

    println!("Print job finished successfully!");

    Ok(())
}
